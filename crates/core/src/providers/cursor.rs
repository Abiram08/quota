//! Cursor included-usage for the current billing period.
//!
//! Verified on Windows:
//! * State DB: `%APPDATA%\Cursor\User\globalStorage\state.vscdb` (SQLite,
//!   WAL), table `ItemTable(key, value)`, keys `cursorAuth/accessToken`,
//!   `cursorAuth/stripeMembershipType`. Opened strictly read-only.
//! * Endpoint: Connect RPC `aiserver.v1.DashboardService/GetCurrentPeriodUsage`
//!   on `https://api2.cursor.sh` (service, method and response fields taken
//!   from Cursor's bundled JS): `billing_cycle_start/end` (int64 ms),
//!   `plan_usage.{total_spend, limit, total_percent_used, auto_percent_used,
//!   api_percent_used}`.
//!
//! NOT verified: a successful response. Recent Cursor builds encrypt the
//! stored token at rest (`cursor.storage.authTokenEncryptionAtRest = true`);
//! Quota does not decrypt another app's secrets, so such installs report
//! "unavailable". Units of `total_spend`/`limit` are assumed to be cents.

use super::{check_jwt_expiry, plan_label};
use crate::json::{any, num_any, ts_any};
use crate::{Limit, Provider, ProviderError, Unit};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

const USAGE_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage";

pub struct Cursor;

pub(crate) fn db_path() -> Option<PathBuf> {
    crate::paths::roaming().map(|r| r.join("Cursor").join("User").join("globalStorage").join("state.vscdb"))
}

fn db_error(e: rusqlite::Error) -> ProviderError {
    use rusqlite::ErrorCode;
    match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            ProviderError::Unavailable("Cursor database is locked; will retry".into())
        }
        _ => ProviderError::Unavailable("cannot read Cursor state database".into()),
    }
}

/// Values may be stored as TEXT or BLOB depending on the writer.
fn read_key(conn: &Connection, key: &str) -> Result<Option<String>, ProviderError> {
    let v: Option<rusqlite::types::Value> = conn
        .query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .map_err(db_error)?;
    Ok(match v {
        Some(rusqlite::types::Value::Text(s)) => Some(s),
        Some(rusqlite::types::Value::Blob(b)) => String::from_utf8(b).ok(),
        _ => None,
    }
    .map(|s| s.trim().trim_matches('"').to_string())
    .filter(|s| !s.is_empty()))
}

struct Stored {
    token: Option<String>,
    membership: Option<String>,
    encrypted_at_rest: bool,
}

fn read_state() -> Result<Stored, ProviderError> {
    let path = db_path().ok_or_else(|| ProviderError::NotConfigured("APPDATA is not set".into()))?;
    if !path.exists() {
        return Err(ProviderError::NotConfigured("Cursor is not installed or never signed in".into()));
    }
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_error)?;
    conn.busy_timeout(Duration::from_millis(1500)).map_err(db_error)?;
    Ok(Stored {
        token: read_key(&conn, "cursorAuth/accessToken")?,
        membership: read_key(&conn, "cursorAuth/stripeMembershipType")?,
        encrypted_at_rest: read_key(&conn, "cursor.storage.authTokenEncryptionAtRest")?.as_deref() == Some("true"),
    })
}

impl Provider for Cursor {
    fn id(&self) -> &'static str {
        "cursor"
    }

    fn name(&self) -> &'static str {
        "Cursor"
    }

    fn fetch(&self) -> Result<Vec<Limit>, ProviderError> {
        let now = crate::time::now();
        let state = read_state()?;
        let token = state
            .token
            .ok_or_else(|| ProviderError::NotConfigured("not signed in to Cursor".into()))?;
        if !crate::json::is_jwt(&token) {
            return Err(ProviderError::Unavailable(if state.encrypted_at_rest {
                "Sign-in token is encrypted; Quota can't read it".into()
            } else {
                "unrecognised Cursor token format".into()
            }));
        }
        check_jwt_expiry(&token, now, "open Cursor to sign in again")?;
        let bearer = format!("Bearer {token}");
        let body = crate::http::post_json(
            USAGE_URL,
            &[("Authorization", &bearer), ("Connect-Protocol-Version", "1")],
            "{}",
        )?;
        parse_usage(&body, state.membership.as_deref().and_then(plan_label).as_deref())
    }
}

/// Parses the Connect JSON body (camelCase per proto3 JSON; snake_case also
/// accepted). Proto3 JSON omits zero values, so missing spend means 0.
pub fn parse_usage(v: &Value, plan: Option<&str>) -> Result<Vec<Limit>, ProviderError> {
    let start = ts_any(v, &["billingCycleStart", "billing_cycle_start"]);
    let end = ts_any(v, &["billingCycleEnd", "billing_cycle_end"]);
    let Some(pu) = any(v, &["planUsage", "plan_usage"]) else {
        return Err(ProviderError::Parse("no plan usage in Cursor response".into()));
    };
    let mk = |label: &str, used: f64, limit: f64, unit: Unit| Limit {
        provider: "cursor".into(),
        account_label: plan.map(str::to_string),
        label: label.into(),
        used: used.max(0.0),
        limit,
        unit,
        window_start: start,
        resets_at: end,
    };
    let mut out = Vec::new();
    let limit = num_any(pu, &["limit"]).unwrap_or(0.0);
    let spend = num_any(pu, &["totalSpend", "total_spend"]).unwrap_or(0.0);
    let total_pct = num_any(pu, &["totalPercentUsed", "total_percent_used"]);
    if limit > 0.0 {
        out.push(mk("Included usage", spend / 100.0, limit / 100.0, Unit::Usd));
    } else if let Some(p) = total_pct {
        out.push(mk("Included usage", p, 100.0, Unit::Percent));
    }
    for (keys, label) in [
        (["autoPercentUsed", "auto_percent_used"], "Auto"),
        (["apiPercentUsed", "api_percent_used"], "API models"),
    ] {
        if let Some(p) = num_any(pu, &keys) {
            out.push(mk(label, p, 100.0, Unit::Percent));
        }
    }
    if out.is_empty() {
        return Err(ProviderError::Parse("Cursor plan usage has no limit".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_connect_json() {
        // int64 as strings, as proto3 JSON encodes them.
        let v = serde_json::json!({
            "billingCycleStart": "1794528000000",
            "billingCycleEnd": "1797120000000",
            "planUsage": {"totalSpend": 1250, "limit": 2000, "autoPercentUsed": 40.5, "unknownField": 1},
            "enabled": true
        });
        let l = parse_usage(&v, Some("Pro")).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].used, l[0].limit, l[0].unit), (12.5, 20.0, Unit::Usd));
        assert_eq!(l[0].window_start, Some(1_794_528_000));
        assert_eq!(l[0].resets_at, Some(1_797_120_000));
        assert_eq!(l[1].label, "Auto");
    }

    #[test]
    fn snake_case_percent_only_and_omitted_zero() {
        let v = serde_json::json!({"billing_cycle_end": 1797120000000i64, "plan_usage": {"total_percent_used": 7}});
        let l = parse_usage(&v, None).unwrap();
        assert_eq!((l[0].used, l[0].unit), (7.0, Unit::Percent));
        // limit set, spend omitted (= 0)
        let v = serde_json::json!({"planUsage": {"limit": 2000}});
        assert_eq!(parse_usage(&v, None).unwrap()[0].used, 0.0);
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_usage(&serde_json::json!({}), None).is_err());
        assert!(parse_usage(&serde_json::json!({"planUsage": {}}), None).is_err());
    }

    #[test]
    fn reads_sqlite_read_only_with_text_and_blob() {
        let dir = std::env::temp_dir().join(format!("quota-cursor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.vscdb");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
                 INSERT INTO ItemTable VALUES ('cursorAuth/stripeMembershipType', 'pro');
                 INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', CAST('a.b.c' AS BLOB));",
            )
            .unwrap();
        }
        let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(read_key(&c, "cursorAuth/stripeMembershipType").unwrap().as_deref(), Some("pro"));
        assert_eq!(read_key(&c, "cursorAuth/accessToken").unwrap().as_deref(), Some("a.b.c"));
        assert_eq!(read_key(&c, "missing").unwrap(), None);
        // Writes are refused on a read-only connection.
        assert!(c.execute("DELETE FROM ItemTable", []).is_err());
        drop(c);
        let _ = std::fs::remove_dir_all(dir);
    }
}
