//! `%LOCALAPPDATA%\quota\snapshot.json`: the last known limits of every
//! provider. Written atomically (temp file + rename) so a reader never sees
//! a half-written file. When a provider fails, its previous limits are kept
//! and flagged `stale` with the failure reason.

use crate::{Limit, ProviderError};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SNAPSHOT_SCHEMA: &str = "quota.snapshot.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema: String,
    /// Time of the most recent refresh of any provider.
    #[serde(with = "crate::time::rfc3339")]
    pub fetched_at: i64,
    pub providers: Vec<ProviderSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub id: String,
    pub name: String,
    /// Time of the last *successful* fetch (None if never succeeded).
    #[serde(default, with = "crate::time::opt_rfc3339")]
    pub fetched_at: Option<i64>,
    /// Time of the last attempt, successful or not.
    #[serde(default, with = "crate::time::opt_rfc3339")]
    pub attempted_at: Option<i64>,
    /// True when `limits` come from an earlier fetch because the latest one failed.
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub error: Option<ErrorInfo>,
    #[serde(default)]
    pub limits: Vec<Limit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub kind: String,
    pub message: String,
}

impl From<&ProviderError> for ErrorInfo {
    fn from(e: &ProviderError) -> Self {
        ErrorInfo { kind: e.kind().into(), message: e.message().into() }
    }
}

impl ProviderSnapshot {
    /// "Unavailable" = nothing to show at all.
    pub fn is_unavailable(&self) -> bool {
        self.limits.is_empty() && self.error.is_some()
    }
}

#[derive(Debug)]
pub enum SnapshotError {
    Missing,
    Io(std::io::Error),
    Invalid(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Missing => f.write_str("no snapshot yet"),
            SnapshotError::Io(e) => write!(f, "snapshot unreadable: {e}"),
            SnapshotError::Invalid(e) => write!(f, "snapshot invalid: {e}"),
        }
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot { schema: SNAPSHOT_SCHEMA.into(), fetched_at: 0, providers: Vec::new() }
    }
}

impl Snapshot {
    pub fn read(path: &Path) -> Result<Snapshot, SnapshotError> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(SnapshotError::Missing),
            Err(e) => return Err(SnapshotError::Io(e)),
        };
        let snap: Snapshot = serde_json::from_slice(&bytes).map_err(|e| SnapshotError::Invalid(e.to_string()))?;
        if snap.schema != SNAPSHOT_SCHEMA {
            return Err(SnapshotError::Invalid(format!("unknown schema {:?}", snap.schema)));
        }
        Ok(snap)
    }

    /// Reads the snapshot at its default location.
    pub fn load() -> Result<Snapshot, SnapshotError> {
        match crate::paths::snapshot_file() {
            Some(path) => Self::read(&path),
            None => Err(SnapshotError::Missing),
        }
    }

    /// Reads the default snapshot; missing or invalid -> empty snapshot.
    pub fn load_or_default() -> Snapshot {
        Self::load().unwrap_or_default()
    }

    /// Atomic write: `snapshot.json.tmp` then rename over the target
    /// (std's rename uses MOVEFILE_REPLACE_EXISTING on Windows).
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = crate::paths::snapshot_file()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))?;
        self.write(&path)
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderSnapshot> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Records one provider's fetch result at `now`.
    pub fn apply(&mut self, id: &str, name: &str, result: Result<Vec<Limit>, ProviderError>, now: i64) {
        let idx = match self.providers.iter().position(|p| p.id == id) {
            Some(i) => i,
            None => {
                self.providers.push(ProviderSnapshot {
                    id: id.into(),
                    name: name.into(),
                    fetched_at: None,
                    attempted_at: None,
                    stale: false,
                    error: None,
                    limits: Vec::new(),
                });
                self.providers.len() - 1
            }
        };
        let p = &mut self.providers[idx];
        p.name = name.into();
        p.attempted_at = Some(now);
        match result {
            Ok(limits) => {
                p.limits = limits;
                p.fetched_at = Some(now);
                p.stale = false;
                p.error = None;
            }
            Err(e) => {
                p.error = Some(ErrorInfo::from(&e));
                // Drop previous limits whose window has already reset: they
                // would show a misleading percentage.
                p.limits.retain(|l| l.resets_at.is_none_or(|r| r > now));
                p.stale = !p.limits.is_empty();
            }
        }
        self.fetched_at = now;
    }

    /// Keeps only providers in `order` and sorts them by it.
    pub fn retain_order(&mut self, order: &[String]) {
        self.providers.retain(|p| order.contains(&p.id));
        self.providers
            .sort_by_key(|p| order.iter().position(|o| *o == p.id).unwrap_or(usize::MAX));
    }

    /// All non-stale-or-stale limits across providers, most urgent first.
    pub fn limits_by_urgency(&self, now: i64) -> Vec<&Limit> {
        let mut v: Vec<&Limit> = self.providers.iter().flat_map(|p| p.limits.iter()).collect();
        v.sort_by(|a, b| crate::pace::urgency(a, now).total_cmp(&crate::pace::urgency(b, now)));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Unit;

    fn lim(label: &str, used: f64, resets_at: Option<i64>) -> Limit {
        Limit {
            provider: "claude".into(),
            account_label: Some("Pro".into()),
            label: label.into(),
            used,
            limit: 100.0,
            unit: Unit::Percent,
            window_start: Some(1_000),
            resets_at,
        }
    }

    fn tmp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("quota-test-{}-{name}", std::process::id()))
            .join("snapshot.json")
    }

    #[test]
    fn write_then_read_round_trip() {
        let path = tmp_path("rt");
        let mut s = Snapshot::default();
        s.apply("claude", "Claude Code", Ok(vec![lim("Session (5h)", 18.0, Some(19_000))]), 1_800);
        s.write(&path).unwrap();
        let back = Snapshot::read(&path).unwrap();
        assert_eq!(back, s);
        // No temp file left behind.
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        // Overwrite works (rename over an existing file).
        s.apply("claude", "Claude Code", Ok(vec![]), 2_000);
        s.write(&path).unwrap();
        assert_eq!(Snapshot::read(&path).unwrap().fetched_at, 2_000);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn timestamps_are_rfc3339_on_disk() {
        let mut s = Snapshot::default();
        s.apply("claude", "Claude Code", Ok(vec![lim("Weekly", 1.0, Some(0))]), 0);
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"fetched_at\":\"1970-01-01T00:00:00Z\""), "{text}");
        assert!(text.contains("\"schema\":\"quota.snapshot.v1\""));
    }

    #[test]
    fn missing_and_invalid() {
        assert!(matches!(Snapshot::read(&tmp_path("none")), Err(SnapshotError::Missing)));
        let path = tmp_path("bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        assert!(matches!(Snapshot::read(&path), Err(SnapshotError::Invalid(_))));
        std::fs::write(&path, br#"{"schema":"other","fetched_at":0,"providers":[]}"#).unwrap();
        assert!(matches!(Snapshot::read(&path), Err(SnapshotError::Invalid(_))));
        std::fs::write(&path, br#"{"schema":"quota.snapshot.v1","fetched_at":"1970-01-01T00:00:00Z","providers":[]}"#)
            .unwrap();
        assert!(Snapshot::read(&path).unwrap().providers.is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn failure_keeps_previous_limits_as_stale() {
        let mut s = Snapshot::default();
        s.apply("claude", "Claude Code", Ok(vec![lim("Session", 18.0, Some(10_000)), lim("Old", 5.0, Some(1_500))]), 1_000);
        s.apply("claude", "Claude Code", Err(ProviderError::Network("timed out".into())), 2_000);
        let p = s.provider("claude").unwrap();
        assert!(p.stale);
        assert_eq!(p.fetched_at, Some(1_000));
        assert_eq!(p.attempted_at, Some(2_000));
        assert_eq!(p.error.as_ref().unwrap().kind, "network");
        // The limit that already reset is dropped, the live one kept.
        assert_eq!(p.limits.len(), 1);
        assert_eq!(p.limits[0].label, "Session");
        // Recovery clears stale + error.
        s.apply("claude", "Claude Code", Ok(vec![lim("Session", 20.0, Some(10_000))]), 3_000);
        let p = s.provider("claude").unwrap();
        assert!(!p.stale && p.error.is_none());
    }

    #[test]
    fn failure_without_history_is_unavailable() {
        let mut s = Snapshot::default();
        s.apply("cursor", "Cursor", Err(ProviderError::NotConfigured("not signed in".into())), 5);
        let p = s.provider("cursor").unwrap();
        assert!(p.is_unavailable());
        assert!(!p.stale);
    }

    #[test]
    fn order_and_urgency() {
        let mut s = Snapshot::default();
        s.apply("codex", "Codex", Ok(vec![lim("Monthly", 88.0, None)]), 1);
        s.apply("claude", "Claude Code", Ok(vec![lim("Session", 18.0, None)]), 1);
        s.apply("copilot", "Copilot", Ok(vec![]), 1);
        s.retain_order(&["claude".into(), "codex".into()]);
        let ids: Vec<_> = s.providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["claude", "codex"]);
        assert_eq!(s.limits_by_urgency(1)[0].label, "Monthly");
    }
}
