//! Pace math: how far through the window we are, where usage is heading, and
//! when the limit runs out if the current rate continues.

use crate::Limit;
use serde::Serialize;

/// Projections below this elapsed fraction are too noisy to warn about.
pub const MIN_ELAPSED_FOR_WARNING: f64 = 0.10;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Pace {
    /// `0..=1` fraction of the window that has elapsed.
    pub elapsed_fraction: f64,
    /// Usage at reset time if the average rate so far continues.
    pub projected: f64,
    /// When `used` reaches `limit` at the current rate, if that is before the
    /// reset. Equals `now` when the limit is already exhausted.
    #[serde(with = "crate::time::opt_rfc3339")]
    pub runs_out_at: Option<i64>,
}

impl Pace {
    /// True when the limit will (or did) run out before it resets.
    pub fn will_run_out(&self) -> bool {
        self.runs_out_at.is_some()
    }
}

/// Elapsed fraction of `[start, end)` at `now`, clamped to `0..=1`.
pub fn elapsed_fraction(start: i64, end: i64, now: i64) -> f64 {
    if end <= start {
        return 1.0;
    }
    ((now - start) as f64 / (end - start) as f64).clamp(0.0, 1.0)
}

/// Pace for a limit; `None` unless both window ends are known.
pub fn compute(limit: &Limit, now: i64) -> Option<Pace> {
    let (start, end) = (limit.window_start?, limit.resets_at?);
    if end <= start {
        return None;
    }
    let elapsed = elapsed_fraction(start, end, now);
    let used = limit.used.max(0.0);
    let projected = if elapsed > 0.0 { used / elapsed } else { used };

    let runs_out_at = if limit.limit > 0.0 && used >= limit.limit {
        Some(now.min(end))
    } else if limit.limit > 0.0 && used > 0.0 && now > start && now < end && projected > limit.limit {
        // Constant rate since window start: used / (now - start) per second.
        let rate = used / (now - start) as f64;
        let t = start as f64 + limit.limit / rate;
        let t = t.round() as i64;
        (t < end).then_some(t.max(now))
    } else {
        None
    };

    Some(Pace { elapsed_fraction: elapsed, projected, runs_out_at })
}

/// Sort key: smaller = more urgent. Exhausted or projected-to-run-out limits
/// come first, then by percent left.
pub fn urgency(limit: &Limit, now: i64) -> f64 {
    let left = limit.percent_left();
    match compute(limit, now) {
        Some(p) if p.will_run_out() && p.elapsed_fraction >= MIN_ELAPSED_FOR_WARNING => left - 1000.0,
        _ => left,
    }
}

/// Burn budget: how fast the remaining amount can be used so that it lasts
/// exactly until reset. Expressed in percent of the limit per hour or per
/// day, whichever reads better.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// Percent of the whole limit per `per` (e.g. 0.5 %/day).
    pub percent: f64,
    pub per: BudgetUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetUnit {
    Hour,
    Day,
}

impl Budget {
    /// `0.5%/day`, `12%/h`, `<0.1%/day`.
    pub fn label(&self) -> String {
        let unit = match self.per {
            BudgetUnit::Hour => "h",
            BudgetUnit::Day => "day",
        };
        let p = self.percent;
        if p < 0.1 {
            format!("<0.1%/{unit}")
        } else if p < 10.0 {
            // One decimal, without a pointless ".0" (`6%/h`, `0.5%/day`).
            let v = (p * 10.0).round() / 10.0;
            if v.fract() == 0.0 {
                format!("{}%/{unit}", v as i64)
            } else {
                format!("{v:.1}%/{unit}")
            }
        } else {
            format!("{}%/{unit}", p.round() as i64)
        }
    }
}

/// `None` when the reset time is unknown, already passed, or nothing is left.
pub fn budget(limit: &Limit, now: i64) -> Option<Budget> {
    let reset = limit.resets_at?;
    let remaining = reset - now;
    let left = limit.percent_left();
    if remaining <= 0 || left <= 0.0 {
        return None;
    }
    let per_hour = left / (remaining as f64 / 3600.0);
    // Windows longer than two days read better per day.
    Some(if remaining > 2 * 86_400 {
        Budget { percent: per_hour * 24.0, per: BudgetUnit::Day }
    } else {
        Budget { percent: per_hour, per: BudgetUnit::Hour }
    })
}

/// Human budget text. With under an hour to go a rate per hour overstates
/// things (`203%/h`), so it reads `62% in 18m` instead.
pub fn budget_label(limit: &Limit, now: i64) -> Option<String> {
    let b = budget(limit, now)?;
    let remaining = limit.resets_at? - now;
    if remaining < 3600 {
        return Some(format!(
            "{}% in {}",
            limit.percent_left().floor() as i64,
            crate::format::duration(remaining)
        ));
    }
    Some(b.label())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Unit;

    const H: i64 = 3600;

    #[test]
    fn budget_label_short_window() {
        // 62% left, 18 minutes to go.
        let l = lim(38.0, 0, 5 * H);
        assert_eq!(budget_label(&l, 5 * H - 18 * 60).as_deref(), Some("62% in 18m"));
        assert_eq!(budget_label(&l, 2 * H).as_deref(), Some("21%/h"));
    }

    #[test]
    fn budget_per_hour_and_day() {
        // 60% left with 3h to go -> 20%/h.
        let b = budget(&lim(40.0, 0, 5 * H), 2 * H).unwrap();
        assert_eq!(b.per, BudgetUnit::Hour);
        assert!((b.percent - 20.0).abs() < 1e-9);
        assert_eq!(b.label(), "20%/h");
        // 12% left with 24 days to go -> 0.5%/day.
        let b = budget(&lim(88.0, 0, 30 * 24 * H), 6 * 24 * H).unwrap();
        assert_eq!(b.per, BudgetUnit::Day);
        assert_eq!(b.label(), "0.5%/day");
        // Exhausted, past reset, or unknown reset -> none.
        assert!(budget(&lim(100.0, 0, 5 * H), H).is_none());
        assert!(budget(&lim(10.0, 0, 5 * H), 6 * H).is_none());
        let mut l = lim(10.0, 0, 5 * H);
        l.resets_at = None;
        assert!(budget(&l, H).is_none());
        assert_eq!(Budget { percent: 0.04, per: BudgetUnit::Day }.label(), "<0.1%/day");
        assert_eq!(Budget { percent: 6.02, per: BudgetUnit::Hour }.label(), "6%/h");
        assert_eq!(Budget { percent: 3.75, per: BudgetUnit::Hour }.label(), "3.8%/h");
    }

    fn lim(used: f64, start: i64, end: i64) -> Limit {
        Limit {
            provider: "t".into(),
            account_label: None,
            label: "Session".into(),
            used,
            limit: 100.0,
            unit: Unit::Percent,
            window_start: Some(start),
            resets_at: Some(end),
        }
    }

    #[test]
    fn elapsed_fraction_clamps() {
        assert_eq!(elapsed_fraction(0, 100, 50), 0.5);
        assert_eq!(elapsed_fraction(0, 100, -5), 0.0);
        assert_eq!(elapsed_fraction(0, 100, 500), 1.0);
        assert_eq!(elapsed_fraction(100, 100, 0), 1.0);
    }

    #[test]
    fn on_pace_does_not_run_out() {
        // 40% used, 50% of the window gone -> projected 80%.
        let p = compute(&lim(40.0, 0, 10 * H), 5 * H).unwrap();
        assert!((p.elapsed_fraction - 0.5).abs() < 1e-9);
        assert!((p.projected - 80.0).abs() < 1e-9);
        assert_eq!(p.runs_out_at, None);
    }

    #[test]
    fn ahead_of_pace_runs_out_before_reset() {
        // 50% used after 2h of a 5h window -> 25%/h -> 100% at 4h.
        let p = compute(&lim(50.0, 0, 5 * H), 2 * H).unwrap();
        assert!((p.projected - 125.0).abs() < 1e-9);
        assert_eq!(p.runs_out_at, Some(4 * H));
        assert!(p.will_run_out());
    }

    #[test]
    fn exhausted_runs_out_now() {
        let p = compute(&lim(100.0, 0, 5 * H), 3 * H).unwrap();
        assert_eq!(p.runs_out_at, Some(3 * H));
    }

    #[test]
    fn exactly_at_limit_projection_is_not_run_out() {
        // projected == limit is fine (runs out exactly at reset).
        let p = compute(&lim(50.0, 0, 10 * H), 5 * H).unwrap();
        assert!((p.projected - 100.0).abs() < 1e-9);
        assert_eq!(p.runs_out_at, None);
    }

    #[test]
    fn window_start_or_now_edge_cases() {
        // At window start nothing can be projected yet.
        let p = compute(&lim(10.0, 0, 5 * H), 0).unwrap();
        assert_eq!(p.elapsed_fraction, 0.0);
        assert_eq!(p.runs_out_at, None);
        // Unknown window -> no pace.
        let mut l = lim(10.0, 0, 5 * H);
        l.window_start = None;
        assert!(compute(&l, H).is_none());
        // Inverted window -> no pace.
        assert!(compute(&lim(10.0, 5 * H, 0), H).is_none());
    }

    #[test]
    fn urgency_prefers_running_out() {
        let calm = lim(80.0, 0, 10 * H); // 20% left, on pace at 9h
        let hot = lim(60.0, 0, 10 * H); // 40% left, but 60% used at 3h
        assert!(urgency(&hot, 3 * H) < urgency(&calm, 9 * H));
    }
}
