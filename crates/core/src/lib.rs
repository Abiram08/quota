//! quota-core: everything that is not UI.
//!
//! * [`Limit`] / [`Provider`] — the provider-agnostic data model.
//! * [`pace`] — elapsed fraction, projected usage, `runs_out_at`.
//! * [`snapshot`] — the on-disk cache shared by the CLI and the tray.
//! * [`config`] — `%APPDATA%\quota\config.toml`.
//! * [`providers`] — Claude Code, Codex, Cursor, GitHub Copilot, OpenRouter.
//!
//! Secrets rule: provider code reads other tools' credentials read-only and
//! never puts token material into a [`ProviderError`], a [`Limit`] or a log.

pub mod backoff;
pub mod config;
pub mod format;
pub mod http;
pub mod json;
pub mod notify;
pub mod output;
pub mod pace;
pub mod paths;
pub mod providers;
pub mod refresh;
pub mod snapshot;
pub mod time;
pub mod tokens;

use serde::{Deserialize, Serialize};
use std::fmt;

/// What `used` / `limit` are measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    /// `limit` is 100 and `used` is a percentage.
    Percent,
    Requests,
    Usd,
    Credits,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Percent => "percent",
            Unit::Requests => "requests",
            Unit::Usd => "usd",
            Unit::Credits => "credits",
        }
    }
}

/// One usage limit of one provider account (e.g. "Claude · Session (5h)").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    /// Provider id, e.g. `"claude"`.
    pub provider: String,
    /// Plan or account label (never an email or token), e.g. `"Pro"`.
    #[serde(default)]
    pub account_label: Option<String>,
    /// Human label of the limit, e.g. `"Session (5h)"`.
    pub label: String,
    pub used: f64,
    pub limit: f64,
    pub unit: Unit,
    /// Unix seconds; `None` when the provider does not expose it.
    #[serde(default, with = "time::opt_rfc3339")]
    pub window_start: Option<i64>,
    /// Unix seconds; `None` when the limit does not reset.
    #[serde(default, with = "time::opt_rfc3339")]
    pub resets_at: Option<i64>,
}

impl Limit {
    /// Fraction used in `0..=1` (can exceed 1 when over the limit).
    pub fn fraction_used(&self) -> f64 {
        if self.limit > 0.0 {
            (self.used / self.limit).max(0.0)
        } else if self.used > 0.0 {
            1.0
        } else {
            0.0
        }
    }

    /// Percent left, clamped to `0..=100`. Rounded to 6 decimals so that
    /// whole numbers stay whole: `(1 - 89/100) * 100` is 10.999999999999996
    /// in floating point, and the floor of that showed 10% instead of 11%.
    pub fn percent_left(&self) -> f64 {
        let v = ((1.0 - self.fraction_used()) * 100.0).clamp(0.0, 100.0);
        (v * 1e6).round() / 1e6
    }

    /// True once the reset time has passed: the stored `used` belongs to a
    /// window that is over, so it must not be shown as current.
    pub fn has_reset(&self, now: i64) -> bool {
        self.resets_at.is_some_and(|r| r <= now)
    }
}

/// Why a provider could not produce limits. Messages are short, user-facing
/// and must never contain secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Tool not installed / never signed in on this machine.
    NotConfigured(String),
    /// Credentials exist but were rejected or are expired.
    Auth(String),
    /// Transport problem, HTTP error status, rate limit.
    Network(String),
    /// Response or local file did not have the expected shape.
    Parse(String),
    /// Anything else that makes the provider unusable right now.
    Unavailable(String),
}

impl ProviderError {
    pub fn message(&self) -> &str {
        match self {
            ProviderError::NotConfigured(m)
            | ProviderError::Auth(m)
            | ProviderError::Network(m)
            | ProviderError::Parse(m)
            | ProviderError::Unavailable(m) => m,
        }
    }

    /// Short machine-readable kind used in JSON output.
    pub fn kind(&self) -> &'static str {
        match self {
            ProviderError::NotConfigured(_) => "not_configured",
            ProviderError::Auth(_) => "auth",
            ProviderError::Network(_) => "network",
            ProviderError::Parse(_) => "parse",
            ProviderError::Unavailable(_) => "unavailable",
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ProviderError {}

/// A usage source. Implementations must be cheap to construct, do all I/O in
/// `fetch`, never panic, and never refresh or modify another tool's tokens.
pub trait Provider: Send + Sync {
    /// Stable id used in config, CLI filters and JSON (`"claude"`).
    fn id(&self) -> &'static str;
    /// Display name (`"Claude Code"`).
    fn name(&self) -> &'static str;
    fn fetch(&self) -> Result<Vec<Limit>, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_percentages_stay_whole() {
        for used in 0..=100 {
            let left = lim(used as f64, 100.0).percent_left();
            assert_eq!(left.floor() as i64, 100 - used, "used {used}");
        }
        assert_eq!(lim(89.0, 100.0).percent_left(), 11.0);
        assert_eq!(lim(3.0, 7.0).percent_left(), 57.142857);
    }

    fn lim(used: f64, limit: f64) -> Limit {
        Limit {
            provider: "t".into(),
            account_label: None,
            label: "l".into(),
            used,
            limit,
            unit: Unit::Requests,
            window_start: None,
            resets_at: None,
        }
    }

    #[test]
    fn percent_left_clamps() {
        assert_eq!(lim(25.0, 100.0).percent_left(), 75.0);
        assert_eq!(lim(150.0, 100.0).percent_left(), 0.0);
        assert_eq!(lim(0.0, 0.0).percent_left(), 100.0);
        assert_eq!(lim(1.0, 0.0).percent_left(), 0.0);
    }
}
