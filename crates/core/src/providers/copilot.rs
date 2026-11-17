//! GitHub Copilot monthly quotas.
//!
//! Verified on Windows:
//! * Token: `%LOCALAPPDATA%\github-copilot\apps.json` (newer) or
//!   `hosts.json` (older): `{"github.com[:app]": {"user", "oauth_token"}}`,
//!   written by Copilot editor plugins.
//! * Endpoint: `GET https://api.github.com/copilot_internal/user` with
//!   `Authorization: token <oauth_token>` returned HTTP 200.
//!
//! NOT verified: the quota fields. The test account has no Copilot plan
//! (`access_type_sku: "no_access"`), so `quota_snapshots`,
//! `quota_reset_date`, `limited_user_quotas` and `monthly_quotas` are parsed
//! tolerantly from their publicly known shape and may need adjusting.

use super::{plan_label, read_json};
use crate::json::{any, at, bool_any, humanize, num_any, str_any, ts_any};
use crate::{Limit, Provider, ProviderError, Unit};
use serde_json::Value;

const USER_URL: &str = "https://api.github.com/copilot_internal/user";

pub struct Copilot;

/// `%LOCALAPPDATA%\github-copilot`, where Copilot editor plugins keep their login.
pub(crate) fn token_dir() -> Option<std::path::PathBuf> {
    crate::paths::local().map(|l| l.join("github-copilot"))
}

fn find_token() -> Result<String, ProviderError> {
    let dir = token_dir().ok_or_else(|| ProviderError::NotConfigured("LOCALAPPDATA is not set".into()))?;
    let mut last_err = ProviderError::NotConfigured("not signed in to GitHub Copilot".into());
    for file in ["apps.json", "hosts.json"] {
        match read_json(&dir.join(file), "not signed in to GitHub Copilot") {
            Ok(v) => {
                if let Some(t) = token_from(&v) {
                    return Ok(t);
                }
            }
            Err(e @ ProviderError::Parse(_)) => last_err = e,
            Err(_) => {}
        }
    }
    // Fallback: OpenCode's GitHub Copilot login (verified to be a `gho_`
    // GitHub OAuth token that copilot_internal/user accepts).
    if let Some(t) = super::opencode_auth().as_ref().and_then(token_from_opencode) {
        return Ok(t);
    }
    Err(last_err)
}

/// `github-copilot.refresh` in OpenCode's `auth.json`. `refresh` holds the
/// GitHub OAuth token; `access` may later hold a short-lived Copilot token.
pub fn token_from_opencode(v: &Value) -> Option<String> {
    let entry = v.get("github-copilot")?;
    ["refresh", "access"]
        .iter()
        .filter_map(|k| str_any(entry, &[k]))
        .find(|t| t.starts_with("gh") && !t.is_empty())
        .map(str::to_string)
}

/// First `oauth_token` under a `github.com` key.
pub fn token_from(v: &Value) -> Option<String> {
    v.as_object()?
        .iter()
        .filter(|(k, _)| k.starts_with("github.com"))
        .find_map(|(_, e)| str_any(e, &["oauth_token"]).filter(|t| !t.is_empty()).map(str::to_string))
}

impl Provider for Copilot {
    fn id(&self) -> &'static str {
        "copilot"
    }

    fn name(&self) -> &'static str {
        "GitHub Copilot"
    }

    fn fetch(&self) -> Result<Vec<Limit>, ProviderError> {
        let token = find_token()?;
        let auth = format!("token {token}");
        let body = crate::http::get_json(USER_URL, &[("Authorization", &auth)])?;
        parse_user(&body)
    }
}

/// One month before `reset` (same day, clamped to month length).
fn month_before(reset: i64) -> i64 {
    use crate::time::{civil_from_days, days_from_civil};
    let days = reset.div_euclid(86_400);
    let secs = reset.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (py, pm) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    let next_first = if pm == 12 { days_from_civil(py + 1, 1, 1) } else { days_from_civil(py, pm + 1, 1) };
    let month_len = (next_first - days_from_civil(py, pm, 1)) as u32;
    days_from_civil(py, pm, d.min(month_len)) * 86_400 + secs
}

fn quota_label(key: &str) -> String {
    match key {
        "premium_interactions" | "premium_requests" => "Premium requests".into(),
        "chat" => "Chat".into(),
        "completions" => "Completions".into(),
        other => humanize(other),
    }
}

pub fn parse_user(v: &Value) -> Result<Vec<Limit>, ProviderError> {
    let plan = str_any(v, &["copilot_plan"]).and_then(plan_label);
    let resets_at = ts_any(v, &["quota_reset_date_utc", "quota_reset_date", "limited_user_reset_date"]);
    let window_start = resets_at.map(month_before);
    let mk = |key: &str, used: f64, limit: f64| Limit {
        provider: "copilot".into(),
        account_label: plan.clone(),
        label: quota_label(key),
        used: used.max(0.0),
        limit,
        unit: Unit::Requests,
        window_start,
        resets_at,
    };
    let mut out = Vec::new();

    // Paid plans: quota_snapshots.{premium_interactions, chat, completions}.
    if let Some(snaps) = at(v, &["quota_snapshots"]).and_then(Value::as_object) {
        for (key, q) in snaps {
            if bool_any(q, &["unlimited"]) == Some(true) {
                continue;
            }
            let Some(ent) = num_any(q, &["entitlement"]).filter(|e| *e > 0.0) else { continue };
            let used = match num_any(q, &["remaining"]) {
                Some(rem) => ent - rem,
                None => match num_any(q, &["percent_remaining"]) {
                    Some(p) => ent * (1.0 - p / 100.0),
                    None => continue,
                },
            };
            out.push(mk(key, used, ent));
        }
    }

    // Free plan: limited_user_quotas (remaining) + monthly_quotas (totals).
    if out.is_empty() {
        if let (Some(rem), Some(tot)) = (
            at(v, &["limited_user_quotas"]).and_then(Value::as_object),
            at(v, &["monthly_quotas"]),
        ) {
            for (key, r) in rem {
                let (Some(r), Some(t)) = (crate::json::num(r), any(tot, &[key.as_str()]).and_then(crate::json::num))
                else {
                    continue;
                };
                if t > 0.0 {
                    out.push(mk(key, t - r, t));
                }
            }
        }
    }

    if out.is_empty() {
        let sku = str_any(v, &["access_type_sku"]).unwrap_or("");
        if sku == "no_access" || plan.is_none() {
            return Err(ProviderError::Unavailable("no active Copilot plan on this GitHub account".into()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_rfc3339;

    #[test]
    fn token_lookup() {
        let v = serde_json::json!({
            "gitlab.com:x": {"oauth_token": "nope"},
            "github.com:Iv1.abc:zed": {"user": "u", "oauth_token": "gho_x"}
        });
        assert_eq!(token_from(&v).as_deref(), Some("gho_x"));
        assert_eq!(token_from(&serde_json::json!({"github.com": {"oauth_token": ""}})), None);
    }

    #[test]
    fn opencode_token_lookup() {
        let v = serde_json::json!({"github-copilot": {"type": "oauth", "refresh": "gho_r", "access": "tid=x", "expires": 0}});
        assert_eq!(token_from_opencode(&v).as_deref(), Some("gho_r"));
        assert_eq!(token_from_opencode(&serde_json::json!({"openrouter": {"key": "k"}})), None);
    }

    #[test]
    fn verified_no_access_response_is_unavailable() {
        let v = serde_json::json!({"login": "someone", "access_type_sku": "no_access", "copilot_plan": "individual", "chat_enabled": false});
        assert_eq!(parse_user(&v).unwrap_err().kind(), "unavailable");
    }

    #[test]
    fn paid_quota_snapshots() {
        let v = serde_json::json!({
            "copilot_plan": "individual",
            "quota_reset_date": "2026-12-01",
            "quota_snapshots": {
                "premium_interactions": {"entitlement": 300, "remaining": 120, "percent_remaining": 40.0, "unlimited": false},
                "chat": {"entitlement": 0, "remaining": 0, "unlimited": true},
                "completions": {"entitlement": 0, "unlimited": true}
            }
        });
        let l = parse_user(&v).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].label.as_str(), l[0].used, l[0].limit), ("Premium requests", 180.0, 300.0));
        assert_eq!(l[0].resets_at, parse_rfc3339("2026-12-01"));
        assert_eq!(l[0].window_start, parse_rfc3339("2026-11-01"));
        assert_eq!(l[0].account_label.as_deref(), Some("Individual"));
    }

    #[test]
    fn free_plan_quotas() {
        let v = serde_json::json!({
            "copilot_plan": "free",
            "limited_user_reset_date": "2026-12-05",
            "limited_user_quotas": {"chat": 40, "completions": 1500},
            "monthly_quotas": {"chat": 50, "completions": 2000}
        });
        let l = parse_user(&v).unwrap();
        assert_eq!(l.len(), 2);
        let chat = l.iter().find(|l| l.label == "Chat").unwrap();
        assert_eq!((chat.used, chat.limit), (10.0, 50.0));
    }

    #[test]
    fn month_before_clamps() {
        let ts = |s| parse_rfc3339(s).unwrap();
        assert_eq!(month_before(ts("2026-03-31T00:00:00Z")), ts("2026-02-28T00:00:00Z"));
        assert_eq!(month_before(ts("2026-01-15T10:00:00Z")), ts("2025-12-15T10:00:00Z"));
    }
}
