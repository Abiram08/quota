//! Human-readable durations, reset times and amounts shared by CLI and tray.

use crate::time::{civil_local, Civil};
use crate::{Limit, Unit};

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Compact count: `2.4M`, `845K`, `912`, `1.2B`.
pub fn count(v: f64) -> String {
    if v >= 1e9 {
        format!("{:.1}B", v / 1e9)
    } else if v >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if v >= 1e3 {
        format!("{:.0}K", v / 1e3)
    } else {
        format!("{}", v.round() as i64)
    }
}

/// Compact duration: `45s`, `12m`, `2h 13m`, `5d 20h`, `23d`.
pub fn duration(secs: i64) -> String {
    let s = secs.max(0);
    let (d, h, m) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60);
    match () {
        _ if d >= 10 => format!("{d}d"),
        _ if d > 0 => format!("{d}d {h}h"),
        _ if h > 0 => format!("{h}h {m}m"),
        _ if m > 0 => format!("{m}m"),
        _ => format!("{s}s"),
    }
}

/// `in 2h 13m` / `5m ago` / `now`.
pub fn relative(ts: i64, now: i64) -> String {
    let d = ts - now;
    if d.abs() < 30 {
        "now".into()
    } else if d > 0 {
        format!("in {}", duration(d))
    } else {
        format!("{} ago", duration(-d))
    }
}

/// Local clock label scaled to distance: `21:59`, `Wed 18:59`, `Dec 6`.
pub fn clock(ts: i64, now: i64) -> String {
    clock_with(civil_local(ts), civil_local(now), ts - now)
}

fn clock_with(t: Civil, n: Civil, delta: i64) -> String {
    let same_day = t.year == n.year && t.month == n.month && t.day == n.day;
    let month = MONTHS.get(t.month.saturating_sub(1) as usize).copied().unwrap_or("?");
    if same_day {
        format!("{:02}:{:02}", t.hour, t.minute)
    } else if delta.abs() < 6 * 86_400 {
        let wd = WEEKDAYS.get(t.weekday as usize).copied().unwrap_or("?");
        format!("{wd} {:02}:{:02}", t.hour, t.minute)
    } else if t.year == n.year {
        format!("{month} {}", t.day)
    } else {
        format!("{month} {}, {}", t.day, t.year)
    }
}

/// `in 2h 13m (21:59)`, or `—` when unknown.
pub fn reset(resets_at: Option<i64>, now: i64) -> String {
    match resets_at {
        Some(ts) => format!("{} ({})", relative(ts, now), clock(ts, now)),
        None => "—".into(),
    }
}

/// Value in a limit's unit: `18%`, `120 req`, `$4.20`, `12 cr`.
pub fn amount(v: f64, unit: Unit) -> String {
    match unit {
        Unit::Percent => format!("{}%", round_pct(v)),
        Unit::Requests => format!("{}", v.round() as i64),
        Unit::Usd => format!("${v:.2}"),
        Unit::Credits => format!("{} cr", v.round() as i64),
    }
}

/// `18%` for percent units, `120 / 300` otherwise.
pub fn usage(l: &Limit) -> String {
    match l.unit {
        Unit::Percent => amount(l.used, l.unit),
        _ => format!("{} / {}", amount(l.used, l.unit), amount(l.limit, l.unit)),
    }
}

/// Percent rounded for display; shows `<1` style precision only via ceil
/// on "used" so 0.4% used does not read as 0%.
pub fn round_pct(v: f64) -> i64 {
    if v > 0.0 && v < 1.0 {
        1
    } else {
        v.round() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::civil_utc;

    #[test]
    fn durations() {
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(12 * 60 + 5), "12m");
        assert_eq!(duration(2 * 3600 + 13 * 60), "2h 13m");
        assert_eq!(duration(5 * 86_400 + 20 * 3600), "5d 20h");
        assert_eq!(duration(23 * 86_400 + 5), "23d");
        assert_eq!(duration(-5), "0s");
    }

    #[test]
    fn relatives() {
        assert_eq!(relative(100, 100), "now");
        assert_eq!(relative(100 + 7200, 100), "in 2h 0m");
        assert_eq!(relative(100, 100 + 300), "5m ago");
    }

    #[test]
    fn clocks() {
        let now = 1_794_528_000; // 2026-11-13T00:00:00Z, Friday
        let n = civil_utc(now);
        assert_eq!(clock_with(civil_utc(now + 3600 + 60), n, 3660), "01:01");
        assert_eq!(clock_with(civil_utc(now + 2 * 86_400), n, 2 * 86_400), "Sun 00:00");
        assert_eq!(clock_with(civil_utc(now + 20 * 86_400), n, 20 * 86_400), "Dec 3");
        assert_eq!(clock_with(civil_utc(now + 60 * 86_400), n, 60 * 86_400), "Jan 12, 2027");
    }

    #[test]
    fn amounts() {
        assert_eq!(amount(18.4, Unit::Percent), "18%");
        assert_eq!(amount(0.3, Unit::Percent), "1%");
        assert_eq!(amount(4.2, Unit::Usd), "$4.20");
        assert_eq!(amount(120.0, Unit::Requests), "120");
    }
}
