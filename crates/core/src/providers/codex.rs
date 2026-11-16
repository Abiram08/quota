//! Codex / ChatGPT plan limits.
//!
//! Verified on Windows with the npm `@openai/codex` package:
//! * Credentials: `%USERPROFILE%\.codex\auth.json` (`CODEX_HOME` overrides)
//!   -> `tokens.{access_token (JWT), account_id}`; `OPENAI_API_KEY` mode has
//!   no plan limits.
//! * Usage: `GET https://chatgpt.com/backend-api/wham/usage` with
//!   `Authorization: Bearer <access_token>` and `chatgpt-account-id`
//!   (path found in codex.exe; returned HTTP 200 on a Free plan).
//!   Body: `plan_type`, `rate_limit.{primary_window, secondary_window}` with
//!   `{used_percent, limit_window_seconds, reset_after_seconds, reset_at}`,
//!   plus `code_review_rate_limit` of the same shape.
//!
//! The token is never refreshed here; Codex refreshes it when it runs.

use super::{check_jwt_expiry, plan_label, read_json};
use crate::json::{at, humanize, num_any, str_any, ts_any};
use crate::{Limit, Provider, ProviderError, Unit};
use serde_json::Value;
use std::path::PathBuf;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

pub struct Codex;

pub(crate) fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::paths::home().map(|h| h.join(".codex")))
}

impl Provider for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn name(&self) -> &'static str {
        "Codex"
    }

    fn fetch(&self) -> Result<Vec<Limit>, ProviderError> {
        let now = crate::time::now();
        let home = codex_home().ok_or_else(|| ProviderError::NotConfigured("USERPROFILE is not set".into()))?;
        let auth = read_json(&home.join("auth.json"), "not signed in to Codex")?;
        let Some(tokens) = at(&auth, &["tokens"]) else {
            return Err(if at(&auth, &["OPENAI_API_KEY"]).is_some() {
                ProviderError::NotConfigured("Codex uses an API key; no plan limits".into())
            } else {
                ProviderError::NotConfigured("not signed in to Codex with ChatGPT".into())
            });
        };
        let token = str_any(tokens, &["access_token"])
            .filter(|t| !t.is_empty())
            .ok_or_else(|| ProviderError::NotConfigured("no Codex access token".into()))?;
        check_jwt_expiry(token, now, "run `codex` to refresh it")?;

        let bearer = format!("Bearer {token}");
        let mut headers = vec![("Authorization", bearer.as_str())];
        if let Some(acct) = str_any(tokens, &["account_id"]).filter(|a| !a.is_empty()) {
            headers.push(("chatgpt-account-id", acct));
        }
        let body = crate::http::get_json(USAGE_URL, &headers)?;
        parse_usage(&body, now)
    }
}

/// `18000` -> `5h`, `604800` -> `Weekly`, `2592000` -> `Monthly`.
fn window_label(secs: Option<i64>) -> String {
    match secs {
        Some(18_000) => "5h".into(),
        Some(86_400) => "Daily".into(),
        Some(604_800) => "Weekly".into(),
        Some(s) if (2_419_200..=2_678_400).contains(&s) => "Monthly".into(),
        Some(s) if s > 0 => format!("{} window", crate::format::duration(s)),
        _ => "Limit".into(),
    }
}

fn window_limit(w: &Value, prefix: &str, plan: Option<&str>, now: i64) -> Option<Limit> {
    let used = num_any(w, &["used_percent", "usedPercent"])?;
    let secs = num_any(w, &["limit_window_seconds", "window_seconds"])
        .or_else(|| num_any(w, &["window_minutes"]).map(|m| m * 60.0))
        .map(|s| s as i64);
    let resets_at = ts_any(w, &["reset_at", "resets_at"])
        .or_else(|| num_any(w, &["reset_after_seconds"]).map(|s| now + s as i64));
    let label = format!("{prefix}{}", window_label(secs));
    Some(Limit {
        provider: "codex".into(),
        account_label: plan.map(str::to_string),
        label: label.trim().to_string(),
        used: used.clamp(0.0, 1000.0),
        limit: 100.0,
        unit: Unit::Percent,
        window_start: match (secs, resets_at) {
            (Some(s), Some(r)) if s > 0 => Some(r - s),
            _ => None,
        },
        resets_at,
    })
}

fn rate_limit_group(rl: &Value, prefix: &str, plan: Option<&str>, now: i64, out: &mut Vec<Limit>) {
    for key in ["primary_window", "secondary_window", "primary", "secondary"] {
        if let Some(w) = at(rl, &[key]) {
            if let Some(l) = window_limit(w, prefix, plan, now) {
                if !out.iter().any(|o| o.label == l.label && o.resets_at == l.resets_at) {
                    out.push(l);
                }
            }
        }
    }
}

pub fn parse_usage(v: &Value, now: i64) -> Result<Vec<Limit>, ProviderError> {
    let plan = str_any(v, &["plan_type"]).and_then(plan_label);
    let plan = plan.as_deref();
    let mut out = Vec::new();
    if let Some(rl) = at(v, &["rate_limit"]) {
        rate_limit_group(rl, "", plan, now, &mut out);
    }
    if let Some(rl) = at(v, &["code_review_rate_limit"]) {
        rate_limit_group(rl, "Code review · ", plan, now, &mut out);
    }
    // Shape not observed yet: accept [{limit_name|name, rate_limit|primary_window...}].
    if let Some(extra) = v.get("additional_rate_limits").and_then(Value::as_array) {
        for item in extra {
            let name = str_any(item, &["limit_name", "name", "limit_id"]).map(humanize);
            let prefix = name.map(|n| format!("{n} · ")).unwrap_or_default();
            let rl = at(item, &["rate_limit"]).unwrap_or(item);
            rate_limit_group(rl, &prefix, plan, now, &mut out);
        }
    }
    if out.is_empty() {
        if at(v, &["rate_limit"]).is_none() && plan.is_some() {
            return Ok(out); // plan without metered limits
        }
        return Err(ProviderError::Parse("no limits in Codex usage response".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/codex_usage.json");

    #[test]
    fn parses_free_plan_monthly_window() {
        let v: Value = serde_json::from_str(FIXTURE).unwrap();
        let l = parse_usage(&v, 0).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].label, "Monthly");
        assert_eq!(l[0].used, 88.0);
        assert_eq!(l[0].account_label.as_deref(), Some("Free"));
        assert_eq!(l[0].resets_at, Some(1_796_511_158));
        assert_eq!(l[0].window_start, Some(1_796_511_158 - 2_592_000));
    }

    #[test]
    fn parses_plus_style_two_windows_and_code_review() {
        let v = serde_json::json!({
            "plan_type": "plus",
            "rate_limit": {
                "primary_window": {"used_percent": 12, "limit_window_seconds": 18000, "reset_after_seconds": 600},
                "secondary_window": {"used_percent": 40.5, "limit_window_seconds": 604800, "reset_at": 2_000_000}
            },
            "code_review_rate_limit": {"primary_window": {"used_percent": 3, "limit_window_seconds": 604800, "reset_at": 2_000_000}},
            "future_field": [1, 2]
        });
        let l = parse_usage(&v, 1_000).unwrap();
        let labels: Vec<_> = l.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["5h", "Weekly", "Code review · Weekly"]);
        assert_eq!(l[0].resets_at, Some(1_600));
        assert_eq!(l[0].window_start, Some(1_600 - 18_000));
    }

    #[test]
    fn empty_or_garbage() {
        assert!(parse_usage(&serde_json::json!({}), 0).is_err());
        assert!(parse_usage(&serde_json::json!({"plan_type": "team"}), 0).unwrap().is_empty());
        assert!(parse_usage(&serde_json::json!({"rate_limit": {"primary_window": {"x": 1}}}), 0).is_err());
    }
}
