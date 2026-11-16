//! Claude Code (Pro/Max subscription limits).
//!
//! Verified on Windows with Claude Code 2.1.321:
//! * Credentials: `%USERPROFILE%\.claude\.credentials.json` ->
//!   `claudeAiOauth.{accessToken, expiresAt(ms), subscriptionType}`
//!   (`CLAUDE_CONFIG_DIR` overrides the directory). Not in Credential Manager.
//! * Usage: `GET https://api.anthropic.com/api/oauth/usage` with
//!   `Authorization: Bearer <accessToken>` and `anthropic-beta: oauth-2025-04-20`
//!   (path, host and beta value taken from the installed claude.exe).
//! * Local cache: `%USERPROFILE%\.claude.json` -> `cachedUsageUtilization`
//!   `{fetchedAtMs, utilization: <same body as the endpoint>}`, written by
//!   Claude Code itself. Used first when fresh: zero network, zero token use.
//!
//! The token is never refreshed here; Claude Code refreshes it when it runs.

use super::{plan_label, read_json};
use crate::json::{any, at, humanize, num, num_any, str_any, ts, ts_any};
use crate::{Limit, Provider, ProviderError, Unit};
use serde_json::Value;
use std::path::PathBuf;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA: &str = "oauth-2025-04-20";
const FIVE_HOURS: i64 = 5 * 3600;
const SEVEN_DAYS: i64 = 7 * 86_400;

pub struct Claude {
    /// Use Claude Code's own cached usage if it is at most this old.
    pub max_cache_age_secs: i64,
}

pub(crate) fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::paths::home().map(|h| h.join(".claude")))
}

fn global_config_file() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join(".claude.json")),
        None => crate::paths::home().map(|h| h.join(".claude.json")),
    }
}

impl Provider for Claude {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn name(&self) -> &'static str {
        "Claude Code"
    }

    fn fetch(&self) -> Result<Vec<Limit>, ProviderError> {
        let now = crate::time::now();
        let dir = config_dir().ok_or_else(|| ProviderError::NotConfigured("USERPROFILE is not set".into()))?;
        let creds = read_json(&dir.join(".credentials.json"), "not signed in to Claude Code")?;
        let oauth = at(&creds, &["claudeAiOauth"])
            .ok_or_else(|| ProviderError::NotConfigured("Claude Code is not signed in with a subscription".into()))?;
        let plan = str_any(oauth, &["subscriptionType"]).and_then(plan_label);

        // 1. Claude Code's own cache, if fresh.
        if let Some(cached) = global_config_file().and_then(|p| read_json(&p, "").ok()) {
            if let Some(limits) = from_local_cache(&cached, plan.as_deref(), now, self.max_cache_age_secs) {
                return Ok(limits);
            }
        }

        // 2. Live endpoint with the existing token (read-only use).
        let token = str_any(oauth, &["accessToken"])
            .filter(|t| !t.is_empty())
            .ok_or_else(|| ProviderError::NotConfigured("no Claude access token".into()))?;
        if let Some(exp) = any(oauth, &["expiresAt"]).and_then(ts) {
            if exp <= now {
                return Err(ProviderError::Auth("token expired; run `claude` to refresh it".into()));
            }
        }
        let auth = format!("Bearer {token}");
        let body = crate::http::get_json(USAGE_URL, &[("Authorization", &auth), ("anthropic-beta", BETA)])
            .map_err(|e| match e {
                ProviderError::Auth(_) => ProviderError::Auth("signed out or token expired; run `claude`".into()),
                other => other,
            })?;
        parse_usage(&body, plan.as_deref())
    }
}

/// Limits from `~/.claude.json` if `cachedUsageUtilization` is fresh enough.
pub fn from_local_cache(global: &Value, plan: Option<&str>, now: i64, max_age: i64) -> Option<Vec<Limit>> {
    let cache = at(global, &["cachedUsageUtilization"])?;
    let fetched = any(cache, &["fetchedAtMs"]).and_then(ts)?;
    if now - fetched > max_age || fetched > now + 300 {
        return None;
    }
    let limits = parse_usage(at(cache, &["utilization"])?, plan).ok()?;
    // A cached window that has already reset is useless.
    limits.iter().all(|l| l.resets_at.is_none_or(|r| r > now)).then_some(limits)
}

/// Parses the `/api/oauth/usage` body. Prefers the `limits[]` array and
/// falls back to the older `five_hour` / `seven_day*` objects.
pub fn parse_usage(v: &Value, plan: Option<&str>) -> Result<Vec<Limit>, ProviderError> {
    let weekly_start = at(v, &["seven_day_breakdown", "window_started_at"]).and_then(ts);
    let mk = |label: String, percent: f64, resets_at: Option<i64>, window: Option<i64>| {
        let window_start = match (window, resets_at) {
            (Some(SEVEN_DAYS), Some(r)) => match weekly_start {
                Some(s) if (r - s - SEVEN_DAYS).abs() < 3600 => Some(s),
                _ => Some(r - SEVEN_DAYS),
            },
            (Some(w), Some(r)) => Some(r - w),
            _ => None,
        };
        Limit {
            provider: "claude".into(),
            account_label: plan.map(str::to_string),
            label,
            used: percent.clamp(0.0, 1000.0),
            limit: 100.0,
            unit: Unit::Percent,
            window_start,
            resets_at,
        }
    };

    let mut out = Vec::new();
    if let Some(items) = v.get("limits").and_then(Value::as_array) {
        for item in items {
            let Some(percent) = num_any(item, &["percent", "utilization"]) else { continue };
            let kind = str_any(item, &["kind"]).unwrap_or("limit");
            let group = str_any(item, &["group"]).unwrap_or("");
            let resets_at = ts_any(item, &["resets_at"]);
            let scope = at(item, &["scope", "model", "display_name"])
                .or_else(|| at(item, &["scope", "surface", "display_name"]))
                .or_else(|| at(item, &["scope", "surface"]))
                .and_then(Value::as_str);
            let (label, window) = match kind {
                "session" => ("Session (5h)".to_string(), Some(FIVE_HOURS)),
                "weekly_all" => ("Weekly".to_string(), Some(SEVEN_DAYS)),
                "weekly_scoped" => (
                    scope.map_or_else(|| "Weekly (scoped)".to_string(), |s| format!("Weekly ({s})")),
                    Some(SEVEN_DAYS),
                ),
                other => {
                    let base = humanize(other);
                    let label = scope.map_or(base.clone(), |s| format!("{base} ({s})"));
                    let window = match group {
                        "session" => Some(FIVE_HOURS),
                        "weekly" => Some(SEVEN_DAYS),
                        _ => None,
                    };
                    (label, window)
                }
            };
            out.push(mk(label, percent, resets_at, window));
        }
    }
    if out.is_empty() {
        for (key, label, window) in [
            ("five_hour", "Session (5h)", FIVE_HOURS),
            ("seven_day", "Weekly", SEVEN_DAYS),
            ("seven_day_opus", "Weekly (Opus)", SEVEN_DAYS),
            ("seven_day_sonnet", "Weekly (Sonnet)", SEVEN_DAYS),
        ] {
            if let Some(obj) = at(v, &[key]) {
                if let Some(p) = obj.get("utilization").and_then(num) {
                    out.push(mk(label.into(), p, ts_any(obj, &["resets_at"]), Some(window)));
                }
            }
        }
    }
    if out.is_empty() {
        return Err(ProviderError::Parse("no limits in Claude usage response".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_rfc3339;

    const FIXTURE: &str = include_str!("../../tests/fixtures/claude_usage.json");

    #[test]
    fn parses_limits_array() {
        let v: Value = serde_json::from_str(FIXTURE).unwrap();
        let l = parse_usage(&v, Some("Pro")).unwrap();
        let labels: Vec<_> = l.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["Session (5h)", "Weekly", "Weekly (Fable)"]);
        assert_eq!(l[0].used, 18.0);
        assert_eq!(l[0].account_label.as_deref(), Some("Pro"));
        let reset = parse_rfc3339("2026-11-12T21:59:59Z").unwrap();
        assert_eq!(l[0].resets_at, Some(reset));
        assert_eq!(l[0].window_start, Some(reset - FIVE_HOURS));
        // Weekly start comes from seven_day_breakdown.window_started_at.
        assert_eq!(l[1].window_start, parse_rfc3339("2026-11-11T19:00:00Z"));
    }

    #[test]
    fn falls_back_to_legacy_objects() {
        let v: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":42.5,"resets_at":"2026-01-01T05:00:00Z"},
                "seven_day":{"utilization":7,"resets_at":null},
                "seven_day_opus":null,"brand_new_field":{"x":1}}"#,
        )
        .unwrap();
        let l = parse_usage(&v, None).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].used, 42.5);
        assert_eq!(l[1].resets_at, None);
        assert_eq!(l[1].window_start, None);
    }

    #[test]
    fn unknown_kinds_and_bad_items_are_tolerated() {
        let v: Value = serde_json::from_str(
            r#"{"limits":[{"kind":"monthly_thing","group":"weekly","percent":"5","resets_at":"2026-01-08T00:00:00Z"},
                          {"kind":"session"},{"nonsense":true}]}"#,
        )
        .unwrap();
        let l = parse_usage(&v, None).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].label, "Monthly thing");
        assert_eq!(l[0].used, 5.0);
        assert!(parse_usage(&serde_json::json!({}), None).is_err());
    }

    #[test]
    fn local_cache_freshness() {
        let usage: Value = serde_json::from_str(FIXTURE).unwrap();
        let fetched_ms = parse_rfc3339("2026-11-12T20:00:00Z").unwrap() * 1000;
        let global = serde_json::json!({"cachedUsageUtilization": {"fetchedAtMs": fetched_ms, "utilization": usage}});
        let now = fetched_ms / 1000 + 60;
        assert!(from_local_cache(&global, None, now, 180).is_some());
        assert!(from_local_cache(&global, None, now + 600, 180).is_none(), "too old");
        // After the session window reset the cache is not used.
        let late = parse_rfc3339("2026-11-12T22:30:00Z").unwrap();
        assert!(from_local_cache(&global, None, late, 1_000_000).is_none());
        assert!(from_local_cache(&serde_json::json!({}), None, now, 180).is_none());
    }
}
