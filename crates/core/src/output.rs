//! Stable, versioned machine output and short text summaries shared by
//! the CLI, the MCP server and the tray tooltip.
//!
//! `quota.limits.v2`: camelCase keys, RFC 3339 UTC timestamps or null.
//! Fields are only ever *added*; a rename or removal bumps the schema.
//! The same document is printed by `quota limits --json` and returned by
//! the MCP tool `limits_list`, so both always mean the same thing.

use crate::format;
use crate::pace;
use crate::snapshot::{ProviderSnapshot, Snapshot};
use crate::Limit;
use serde::Serialize;

pub const LIMITS_SCHEMA: &str = "quota.limits.v2";

/// RFC 3339 timestamp or null.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ts(pub Option<i64>);

impl Serialize for Ts {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        crate::time::opt_rfc3339::serialize(&self.0, s)
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// `ok` | `stale` | `unavailable`
pub fn status(p: &ProviderSnapshot) -> &'static str {
    if p.is_unavailable() {
        "unavailable"
    } else if p.stale {
        "stale"
    } else {
        "ok"
    }
}

/// The limits document. Field order is part of the contract.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitsDoc {
    pub schema: &'static str,
    pub generated_at: Ts,
    /// When the snapshot was last written (null: no snapshot yet).
    pub updated_at: Ts,
    pub summary: SummaryOut,
    /// Every provider shown, with its state. Limits are listed separately.
    pub providers: Vec<ProviderOut>,
    /// Every limit of the providers shown, in provider order.
    pub limits: Vec<LimitOut>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryOut {
    pub headline: String,
    pub detail: String,
    pub hint: Option<String>,
    /// `ok` | `low` | `critical` | `none`
    pub level: &'static str,
    /// A limit is projected to run out before it resets.
    pub running_out: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOut {
    pub id: String,
    pub name: String,
    /// `ok` | `stale` | `unavailable`
    pub status: &'static str,
    pub fetched_at: Ts,
    pub attempted_at: Ts,
    pub error: Option<ErrorOut>,
}

#[derive(Debug, Serialize)]
pub struct ErrorOut {
    /// `auth` | `network` | `not_configured` | `parse` | `unavailable`
    pub reason: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitOut {
    /// Provider id (`claude`, `codex`, ...).
    pub provider: String,
    pub name: String,
    /// Plan or account label (never an email), e.g. `Pro`.
    pub plan: Option<String>,
    /// `percent` | `usd` | `requests` | ...
    pub unit: &'static str,
    pub used: f64,
    pub limit: f64,
    pub percent_used: f64,
    pub percent_left: f64,
    /// The reset time has passed; `used` is from the previous window.
    pub window_ended: bool,
    pub window_start: Ts,
    pub resets_at: Ts,
    /// Words for people and agents: `Out in 1h 0m`, `Used up`, `0.5%/day`.
    pub status: String,
    pub pace: Option<PaceOut>,
    /// Rate at which the rest can be used and still last until reset.
    pub budget: Option<BudgetOut>,
}

#[derive(Debug, Serialize)]
pub struct BudgetOut {
    pub percent: f64,
    /// `hour` | `day`
    pub per: &'static str,
    pub label: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaceOut {
    pub elapsed_fraction: f64,
    pub projected: f64,
    pub runs_out_at: Ts,
    pub will_run_out: bool,
}

pub fn level_str(l: Level) -> &'static str {
    match l {
        Level::Ok => "ok",
        Level::Low => "low",
        Level::Critical => "critical",
        Level::None => "none",
    }
}

pub fn limit_out(l: &Limit, now: i64) -> LimitOut {
    LimitOut {
        provider: l.provider.clone(),
        name: l.label.clone(),
        plan: l.account_label.clone(),
        unit: l.unit.as_str(),
        used: round2(l.used),
        limit: round2(l.limit),
        percent_used: round2((l.fraction_used() * 100.0).min(100.0)),
        percent_left: round2(l.percent_left()),
        window_ended: l.has_reset(now),
        window_start: Ts(l.window_start),
        resets_at: Ts(l.resets_at),
        status: limit_status(l, now).0,
        budget: pace::budget(l, now).map(|b| BudgetOut {
            percent: (b.percent * 100.0).round() / 100.0,
            per: match b.per {
                pace::BudgetUnit::Hour => "hour",
                pace::BudgetUnit::Day => "day",
            },
            label: pace::budget_label(l, now).unwrap_or_else(|| b.label()),
        }),
        pace: pace::compute(l, now).map(|p| PaceOut {
            elapsed_fraction: (p.elapsed_fraction * 10_000.0).round() / 10_000.0,
            projected: round2(p.projected),
            runs_out_at: Ts(p.runs_out_at),
            will_run_out: p.will_run_out(),
        }),
    }
}

pub fn provider_out(p: &ProviderSnapshot) -> ProviderOut {
    ProviderOut {
        id: p.id.clone(),
        name: p.name.clone(),
        status: status(p),
        fetched_at: Ts(p.fetched_at),
        attempted_at: Ts(p.attempted_at),
        error: p.error.as_ref().map(|e| ErrorOut { reason: e.kind.clone(), message: e.message.clone() }),
    }
}

/// The snapshot narrowed to `only` (provider ids; None = all), in order.
pub fn scoped(snap: &Snapshot, only: Option<&[String]>) -> Snapshot {
    Snapshot {
        providers: snap
            .providers
            .iter()
            .filter(|p| only.is_none_or(|ids| ids.contains(&p.id)))
            .cloned()
            .collect(),
        ..snap.clone()
    }
}

/// The limits document for a snapshot already narrowed with [`scoped`].
pub fn limits_json(snap: &Snapshot, now: i64) -> LimitsDoc {
    let sm = summary(snap, now);
    LimitsDoc {
        schema: LIMITS_SCHEMA,
        generated_at: Ts(Some(now)),
        updated_at: Ts(Some(snap.fetched_at).filter(|t| *t > 0)),
        summary: SummaryOut {
            headline: sm.headline,
            detail: sm.detail,
            hint: sm.hint,
            level: level_str(sm.level),
            running_out: sm.alert,
        },
        providers: snap.providers.iter().map(provider_out).collect(),
        limits: snap.providers.iter().flat_map(|p| p.limits.iter().map(|l| limit_out(l, now))).collect(),
    }
}

/// `no active plan` -> `No active plan` (provider messages are lowercase
/// fragments; shown alone they read as sentences).
pub fn sentence_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Severity shared by the popup, tray icon and CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Low,
    Critical,
    /// No data, or a window that already ended.
    None,
}

impl Level {
    /// 30% left or less is low, 10% or less critical.
    pub fn for_left(left: f64) -> Level {
        if left <= 10.0 {
            Level::Critical
        } else if left <= 30.0 {
            Level::Low
        } else {
            Level::Ok
        }
    }
}

/// The one-sentence summary at the top of the popup and the CLI.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub headline: String,
    pub detail: String,
    pub hint: Option<String>,
    pub level: Level,
    /// True only when a limit is projected to run out before it resets
    /// (the popup shows its banner only then).
    pub alert: bool,
}

fn running_out(l: &Limit, now: i64) -> Option<pace::Pace> {
    pace::compute(l, now).filter(|p| p.will_run_out() && p.elapsed_fraction >= pace::MIN_ELAPSED_FOR_WARNING)
}

fn summary_for(name: &str, l: &Limit, now: i64) -> Summary {
    let left = l.percent_left();
    let left_i = left.floor() as i64;
    let budget = pace::budget_label(l, now);
    let reset_rel = l.resets_at.map(|r| format::relative(r, now));
    if left <= 0.0 {
        return Summary {
            headline: format!("{name} {} is used up", l.label),
            detail: match &reset_rel {
                Some(r) => format!("It resets {r}."),
                None => "No reset date reported.".into(),
            },
            hint: None,
            level: Level::Critical,
            alert: false,
        };
    }
    if let Some(p) = running_out(l, now) {
        let level = Level::for_left(left);
        return Summary {
            headline: format!("{name} runs out in {}", format::duration(p.runs_out_at.unwrap_or(now) - now)),
            detail: match &reset_rel {
                Some(r) => format!("{left_i}% of {} left, and it resets {r}.", l.label),
                None => format!("{left_i}% of {} left.", l.label),
            },
            hint: budget.map(|b| format!("Stay under {b} to make it to the reset.")),
            // Running out is never the calm colour.
            level: if level == Level::Ok { Level::Low } else { level },
            alert: true,
        };
    }
    Summary {
        headline: "Every limit is on pace".into(),
        detail: format!("Lowest is {name} {}, with {left_i}% left.", l.label),
        hint: match (&reset_rel, budget) {
            (Some(r), Some(b)) => Some(format!("Resets {r}. Budget {b}.")),
            (Some(r), None) => Some(format!("Resets {r}.")),
            _ => None,
        },
        level: Level::for_left(left),
        alert: false,
    }
}

/// Summary for the whole snapshot.
pub fn summary(snap: &Snapshot, now: i64) -> Summary {
    match most_urgent(snap, now, 1).into_iter().next() {
        Some((name, limit)) => summary_for(&name, limit, now),
        None if snap.providers.is_empty() => Summary {
            headline: "Fetching usage…".into(),
            detail: "The first refresh is running.".into(),
            hint: None,
            level: Level::None,
            alert: false,
        },
        None => Summary {
            headline: "No limits to show".into(),
            detail: "Sign in to Claude Code, Codex, Cursor, Copilot or OpenRouter.".into(),
            hint: Some("Then refresh.".into()),
            level: Level::None,
            alert: false,
        },
    }
}

/// Right-hand status of a limit: the budget, or a warning in words.
/// Returns `(text, is_warning, level)`.
pub fn limit_status(l: &Limit, now: i64) -> (String, bool, Level) {
    if l.has_reset(now) {
        return ("Refresh to update".into(), false, Level::None);
    }
    let left = l.percent_left();
    let level = Level::for_left(left);
    if left <= 0.0 {
        return ("Used up".into(), true, Level::Critical);
    }
    if let Some(p) = running_out(l, now) {
        let warn = if level == Level::Ok { Level::Low } else { level };
        return (format!("Out in {}", format::duration(p.runs_out_at.unwrap_or(now) - now)), true, warn);
    }
    (pace::budget_label(l, now).unwrap_or_default(), false, level)
}

/// Plain-text summary for the clipboard: one line per current limit, then
/// tools that are not connected. No colour codes, no secrets.
pub fn plain_summary(snap: &Snapshot, now: i64) -> String {
    let mut out = vec![format!("Quota, {}", crate::time::format_rfc3339(now))];
    for p in &snap.providers {
        let name = short_name(&p.id, &p.name);
        if p.is_unavailable() {
            let why = p.error.as_ref().map(|e| e.message.as_str()).unwrap_or("unavailable");
            out.push(format!("{name}: not connected ({why})"));
            continue;
        }
        for l in &p.limits {
            if l.has_reset(now) {
                out.push(format!("{name} {}: reset, refresh to update", l.label));
            } else {
                out.push(summary_line(&name, l, now));
            }
        }
    }
    out.join("\r\n")
}

/// `Claude Session (5h): 82% left, resets in 2h 13m`
pub fn summary_line(provider_name: &str, l: &Limit, now: i64) -> String {
    let mut s = format!("{provider_name} {}: {}% left", l.label, l.percent_left().floor() as i64);
    if let Some(p) = pace::compute(l, now) {
        if p.will_run_out() && p.elapsed_fraction >= pace::MIN_ELAPSED_FOR_WARNING && l.percent_left() > 0.0 {
            if let Some(at) = p.runs_out_at {
                s.push_str(&format!(", out {}", format::relative(at, now)));
                return s;
            }
        }
    }
    if let Some(r) = l.resets_at {
        s.push_str(&format!(", resets {}", format::relative(r, now)));
    }
    s
}

/// Short provider name used in compact text (tooltip).
pub fn short_name(id: &str, name: &str) -> String {
    match id {
        "claude" => "Claude".into(),
        "copilot" => "Copilot".into(),
        _ => name.to_string(),
    }
}

/// The most urgent limits as `(provider_display_name, limit)`.
pub fn most_urgent(snap: &Snapshot, now: i64, n: usize) -> Vec<(String, &Limit)> {
    let mut v: Vec<(String, &Limit)> = snap
        .providers
        .iter()
        .flat_map(|p| p.limits.iter().map(move |l| (short_name(&p.id, &p.name), l)))
        .filter(|(_, l)| !l.has_reset(now))
        .collect();
    v.sort_by(|a, b| pace::urgency(a.1, now).total_cmp(&pace::urgency(b.1, now)));
    v.truncate(n);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProviderError, Unit};

    fn snap() -> Snapshot {
        let mut s = Snapshot::default();
        s.apply(
            "claude",
            "Claude Code",
            Ok(vec![Limit {
                provider: "claude".into(),
                account_label: Some("Pro".into()),
                label: "Session (5h)".into(),
                used: 50.0,
                limit: 100.0,
                unit: Unit::Percent,
                window_start: Some(0),
                resets_at: Some(5 * 3600),
            }]),
            3600,
        );
        s.apply("cursor", "Cursor", Err(ProviderError::Auth("signed out".into())), 3600);
        s
    }

    #[test]
    fn v2_shape_is_stable() {
        let doc = limits_json(&snap(), 3600);
        let text = serde_json::to_string(&doc).unwrap();
        assert!(text.starts_with(r#"{"schema":"quota.limits.v2","generatedAt":"#), "{text}");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["updatedAt"], "1970-01-01T01:00:00Z");
        assert_eq!(v["summary"]["headline"], "Claude runs out in 1h 0m");
        assert_eq!(v["summary"]["level"], "low");
        assert_eq!(v["summary"]["runningOut"], true);
        assert_eq!(v["providers"][0]["status"], "ok");
        let l = &v["limits"][0];
        assert_eq!(l["provider"], "claude");
        assert_eq!(l["name"], "Session (5h)");
        assert_eq!(l["plan"], "Pro");
        assert_eq!(l["percentLeft"], 50.0);
        assert_eq!(l["unit"], "percent");
        assert_eq!(l["resetsAt"], "1970-01-01T05:00:00Z");
        assert_eq!(l["status"], "Out in 1h 0m");
        assert_eq!(l["pace"]["willRunOut"], true);
        assert_eq!(l["pace"]["runsOutAt"], "1970-01-01T02:00:00Z");
        let cur = &v["providers"][1];
        assert_eq!(cur["status"], "unavailable");
        assert_eq!(cur["error"]["reason"], "auth");
        assert!(!text.contains("_at\"") && !text.contains("percent_"), "camelCase only: {text}");
    }

    #[test]
    fn scope_and_empty() {
        let only = vec!["cursor".to_string()];
        let doc = limits_json(&scoped(&snap(), Some(&only)), 0);
        assert_eq!((doc.providers.len(), doc.limits.len()), (1, 0));
        let v = serde_json::to_value(limits_json(&Snapshot::default(), 0)).unwrap();
        assert!(v["updatedAt"].is_null());
        assert!(v["providers"].as_array().unwrap().is_empty());
        assert!(v["limits"].as_array().unwrap().is_empty());
    }

    #[test]
    fn summary() {
        let s = snap();
        let top = most_urgent(&s, 3600, 3);
        assert_eq!(top.len(), 1);
        assert_eq!(summary_line(&top[0].0, top[0].1, 3600), "Claude Session (5h): 50% left, out in 1h 0m");
    }
}
