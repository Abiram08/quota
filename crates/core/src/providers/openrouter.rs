//! OpenRouter credits and limits.
//!
//! Verified on Windows:
//! * Key, first found wins: `OPENROUTER_API_KEY`; then OpenCode's
//!   `auth.json` -> `openrouter.key` (`%USERPROFILE%\.local\share\opencode`,
//!   or `XDG_DATA_HOME\opencode`). Quota stores no key of its own.
//! * `GET https://openrouter.ai/api/v1/key` (documented) returned HTTP 200:
//!   `data.{limit, limit_remaining, limit_reset, usage, usage_daily,
//!   usage_weekly, usage_monthly, is_free_tier,
//!   free_model_daily_requests.{used, limit, remaining}}`.
//! * `GET https://openrouter.ai/api/v1/credits` (documented):
//!   `data.{total_credits, total_usage}` in USD.
//!
//! Assumption (not verifiable without waiting a day): free-model daily
//! requests reset at 00:00 UTC.

use crate::json::{at, num_any, str_any};
use crate::{Limit, Provider, ProviderError, Unit};
use serde_json::Value;

const KEY_URL: &str = "https://openrouter.ai/api/v1/key";
const CREDITS_URL: &str = "https://openrouter.ai/api/v1/credits";

pub struct OpenRouter;

fn env_key() -> Option<String> {
    std::env::var("OPENROUTER_API_KEY").ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

fn opencode_key() -> Option<String> {
    super::opencode_auth()
        .as_ref()
        .and_then(|v| at(v, &["openrouter"]))
        .and_then(|e| str_any(e, &["key"]))
        .filter(|k| !k.is_empty())
        .map(str::to_string)
}

fn find_key() -> Result<String, ProviderError> {
    env_key().or_else(opencode_key).ok_or_else(|| {
        ProviderError::NotConfigured("no OpenRouter key (set OPENROUTER_API_KEY, or sign in to OpenRouter in OpenCode)".into())
    })
}

impl Provider for OpenRouter {
    fn id(&self) -> &'static str {
        "openrouter"
    }

    fn name(&self) -> &'static str {
        "OpenRouter"
    }

    fn fetch(&self) -> Result<Vec<Limit>, ProviderError> {
        let key = find_key()?;
        let auth = format!("Bearer {key}");
        let key_info = crate::http::get_json(KEY_URL, &[("Authorization", &auth)])?;
        // Credits are optional extra detail; a failure there is not fatal.
        let credits = crate::http::get_json(CREDITS_URL, &[("Authorization", &auth)]).ok();
        parse(&key_info, credits.as_ref(), crate::time::now())
    }
}

fn next_utc_midnight(now: i64) -> i64 {
    (now.div_euclid(86_400) + 1) * 86_400
}

/// Start/end of the window named by `limit_reset` (`daily`/`weekly`/`monthly`, UTC).
fn reset_window(kind: &str, now: i64) -> (Option<i64>, Option<i64>) {
    use crate::time::{civil_from_days, days_from_civil};
    let day = now.div_euclid(86_400);
    match kind {
        "daily" => (Some(day * 86_400), Some((day + 1) * 86_400)),
        "weekly" => {
            // ISO weeks start Monday; 1970-01-01 was a Thursday.
            let monday = day - (day + 3).rem_euclid(7);
            (Some(monday * 86_400), Some((monday + 7) * 86_400))
        }
        "monthly" => {
            let (y, m, _) = civil_from_days(day);
            let start = days_from_civil(y, m, 1);
            let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
            (Some(start * 86_400), Some(next * 86_400))
        }
        _ => (None, None),
    }
}

pub fn parse(key: &Value, credits: Option<&Value>, now: i64) -> Result<Vec<Limit>, ProviderError> {
    let d = at(key, &["data"]).unwrap_or(key);
    if !d.is_object() {
        return Err(ProviderError::Parse("unexpected OpenRouter key response".into()));
    }
    let free = d.get("is_free_tier").and_then(Value::as_bool) == Some(true);
    let plan = Some(if free { "Free tier" } else { "Paid" }.to_string());
    let mk = |label: &str, used: f64, limit: f64, unit: Unit, window: (Option<i64>, Option<i64>)| Limit {
        provider: "openrouter".into(),
        account_label: plan.clone(),
        label: label.into(),
        used: used.max(0.0),
        limit,
        unit,
        window_start: window.0,
        resets_at: window.1,
    };
    let mut out = Vec::new();

    // Key spending limit, if one is set.
    if let Some(limit) = num_any(d, &["limit"]).filter(|l| *l > 0.0) {
        let kind = str_any(d, &["limit_reset"]).unwrap_or("");
        let used = match kind {
            "daily" => num_any(d, &["usage_daily"]),
            "weekly" => num_any(d, &["usage_weekly"]),
            "monthly" => num_any(d, &["usage_monthly"]),
            _ => None,
        }
        .or_else(|| num_any(d, &["limit_remaining"]).map(|r| limit - r))
        .or_else(|| num_any(d, &["usage"]))
        .unwrap_or(0.0);
        out.push(mk("Key limit", used, limit, Unit::Usd, reset_window(kind, now)));
    }

    // Free-model daily request allowance.
    if let Some(f) = at(d, &["free_model_daily_requests"]) {
        if let Some(lim) = num_any(f, &["limit"]).filter(|l| *l > 0.0) {
            let used = num_any(f, &["used"])
                .or_else(|| num_any(f, &["remaining"]).map(|r| lim - r))
                .unwrap_or(0.0);
            let reset = next_utc_midnight(now);
            out.push(mk("Free models (daily)", used, lim, Unit::Requests, (Some(reset - 86_400), Some(reset))));
        }
    }

    // Prepaid credits.
    if let Some(c) = credits.map(|c| at(c, &["data"]).unwrap_or(c)) {
        let total = num_any(c, &["total_credits"]).unwrap_or(0.0);
        if total > 0.0 {
            let used = num_any(c, &["total_usage"]).unwrap_or(0.0);
            out.push(mk("Credits", used, total, Unit::Usd, (None, None)));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_rfc3339;

    const KEY: &str = include_str!("../../tests/fixtures/openrouter_key.json");

    #[test]
    fn verified_free_tier_response() {
        let v: Value = serde_json::from_str(KEY).unwrap();
        let now = parse_rfc3339("2026-11-12T20:00:00Z").unwrap();
        let credits = serde_json::json!({"data": {"total_credits": 0, "total_usage": 0.00145365}});
        let l = parse(&v, Some(&credits), now).unwrap();
        assert_eq!(l.len(), 1, "no key limit, no prepaid credits");
        assert_eq!(l[0].label, "Free models (daily)");
        assert_eq!((l[0].used, l[0].limit, l[0].unit), (3.0, 50.0, Unit::Requests));
        assert_eq!(l[0].resets_at, parse_rfc3339("2026-11-13T00:00:00Z"));
        assert_eq!(l[0].account_label.as_deref(), Some("Free tier"));
    }

    #[test]
    fn key_limit_and_credits() {
        let v = serde_json::json!({"data": {"limit": 10, "limit_reset": "monthly", "usage_monthly": 2.5, "is_free_tier": false}});
        let c = serde_json::json!({"data": {"total_credits": 20, "total_usage": 5}});
        let now = parse_rfc3339("2026-11-12T20:00:00Z").unwrap();
        let l = parse(&v, Some(&c), now).unwrap();
        assert_eq!(l[0].label, "Key limit");
        assert_eq!((l[0].used, l[0].limit), (2.5, 10.0));
        assert_eq!(l[0].window_start, parse_rfc3339("2026-11-01"));
        assert_eq!(l[0].resets_at, parse_rfc3339("2026-12-01"));
        assert_eq!(l[1].label, "Credits");
        assert_eq!((l[1].used, l[1].limit), (5.0, 20.0));
    }

    #[test]
    fn weekly_window_starts_monday() {
        let now = parse_rfc3339("2026-11-12T20:00:00Z").unwrap(); // Thursday
        let (s, e) = reset_window("weekly", now);
        assert_eq!(s, parse_rfc3339("2026-11-09"));
        assert_eq!(e, parse_rfc3339("2026-11-16"));
        assert_eq!(reset_window("never", now), (None, None));
    }

    #[test]
    fn garbage() {
        assert!(parse(&serde_json::json!("x"), None, 0).is_err());
        assert!(parse(&serde_json::json!({"data": {}}), None, 0).unwrap().is_empty());
    }
}
