//! Decides which notifications to show, once per limit window. State is
//! persisted (`notified.json`) so restarts and autostart do not repeat them.

use crate::config::Notifications;
use crate::pace::{compute, MIN_ELAPSED_FOR_WARNING};
use crate::snapshot::Snapshot;
use crate::Limit;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub title: String,
    pub body: String,
}

/// key (provider|label|window) -> fired markers ("t30", "t10", "pace").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tracker {
    #[serde(default)]
    fired: BTreeMap<String, Vec<String>>,
    /// Reset times per key so expired entries can be pruned.
    #[serde(default)]
    expires: BTreeMap<String, i64>,
}

/// Window identity. Reset times jitter by fractions of a second between
/// fetches (Claude), so they are rounded to 10 minutes.
fn key(l: &Limit) -> String {
    let window = l.resets_at.map(|r| (r + 300).div_euclid(600)).unwrap_or(0);
    format!("{}|{}|{}", l.provider, l.label, window)
}

impl Tracker {
    pub fn load(path: &Path) -> Tracker {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    fn mark(&mut self, k: &str, marker: String, expires: Option<i64>) -> bool {
        let list = self.fired.entry(k.to_string()).or_default();
        if list.contains(&marker) {
            return false;
        }
        list.push(marker);
        // Limits without a reset time are never pruned.
        self.expires.insert(k.to_string(), expires.unwrap_or(i64::MAX));
        true
    }

    fn prune(&mut self, now: i64) {
        let dead: Vec<String> = self.expires.iter().filter(|(_, &e)| e < now).map(|(k, _)| k.clone()).collect();
        for k in dead {
            self.expires.remove(&k);
            self.fired.remove(&k);
        }
    }

    /// Alerts for limits that newly crossed a threshold or a pace warning.
    /// Stale data never alerts. Only the lowest newly-crossed threshold per
    /// limit fires, so a jump from 50% to 5% left shows a single toast.
    pub fn evaluate(&mut self, snap: &Snapshot, cfg: &Notifications, now: i64) -> Vec<Alert> {
        self.prune(now);
        let mut out = Vec::new();
        if !cfg.enabled {
            return out;
        }
        for p in snap.providers.iter().filter(|p| !p.stale) {
            for l in p.limits.iter().filter(|l| !l.has_reset(now)) {
                let k = key(l);
                let left = l.percent_left();
                let mut crossed = false;
                for &t in &cfg.thresholds_percent_left {
                    if left <= t && self.mark(&k, format!("t{t}"), l.resets_at) {
                        crossed = true;
                    }
                }
                let reset = crate::format::reset(l.resets_at, now);
                if crossed {
                    // A threshold toast already says "running low"; do not
                    // follow it with a separate pace toast for this window.
                    self.mark(&k, "pace".into(), l.resets_at);
                    out.push(Alert {
                        title: format!("{} · {}: {:.0}% left", p.name, l.label, left.floor()),
                        body: format!("Resets {reset}."),
                    });
                    continue;
                }
                if cfg.pace_warnings {
                    if let Some(pace) = compute(l, now) {
                        if let Some(at) = pace.runs_out_at {
                            if pace.elapsed_fraction >= MIN_ELAPSED_FOR_WARNING
                                && left > 0.0
                                && self.mark(&k, "pace".into(), l.resets_at)
                            {
                                out.push(Alert {
                                    title: format!("{} · {}: on pace to run out", p.name, l.label),
                                    body: format!(
                                        "{:.0}% left; runs out {} at this rate, resets {reset}.",
                                        left.floor(),
                                        crate::format::relative(at, now)
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProviderError, Unit};

    const H: i64 = 3600;

    fn snap(used: f64, now: i64) -> Snapshot {
        let mut s = Snapshot::default();
        s.apply(
            "claude",
            "Claude Code",
            Ok(vec![Limit {
                provider: "claude".into(),
                account_label: None,
                label: "Session (5h)".into(),
                used,
                limit: 100.0,
                unit: Unit::Percent,
                window_start: Some(0),
                resets_at: Some(5 * H),
            }]),
            now,
        );
        s
    }

    #[test]
    fn thresholds_fire_once_each() {
        let cfg = Notifications::default();
        let mut t = Tracker::default();
        assert!(t.evaluate(&snap(50.0, 4 * H), &cfg, 4 * H).is_empty());
        let a = t.evaluate(&snap(72.0, 4 * H), &cfg, 4 * H);
        assert_eq!(a.len(), 1);
        assert!(a[0].title.contains("28% left"), "{}", a[0].title);
        assert!(t.evaluate(&snap(75.0, 4 * H), &cfg, 4 * H).is_empty());
        assert_eq!(t.evaluate(&snap(91.0, 4 * H), &cfg, 4 * H).len(), 1);
        assert!(t.evaluate(&snap(95.0, 4 * H), &cfg, 4 * H).is_empty());
    }

    #[test]
    fn big_jump_is_single_alert() {
        let mut t = Tracker::default();
        let a = t.evaluate(&snap(95.0, 4 * H), &Notifications::default(), 4 * H);
        assert_eq!(a.len(), 1);
        assert!(t.evaluate(&snap(96.0, 4 * H), &Notifications::default(), 4 * H).is_empty());
    }

    #[test]
    fn pace_warning() {
        let mut t = Tracker::default();
        // 50% used after 1h of 5h -> runs out at 2h.
        let a = t.evaluate(&snap(50.0, H), &Notifications::default(), H);
        assert_eq!(a.len(), 1);
        assert!(a[0].title.contains("on pace"));
        assert!(t.evaluate(&snap(55.0, H), &Notifications::default(), H).is_empty());
        // Too early in the window: no warning.
        let mut t = Tracker::default();
        assert!(t.evaluate(&snap(10.0, 60), &Notifications::default(), 60).is_empty());
    }

    #[test]
    fn disabled_stale_and_pruned() {
        let mut t = Tracker::default();
        let off = Notifications { enabled: false, ..Default::default() };
        assert!(t.evaluate(&snap(99.0, H), &off, H).is_empty());
        let mut s = snap(99.0, H);
        s.apply("claude", "Claude Code", Err(ProviderError::Network("x".into())), H);
        assert!(t.evaluate(&s, &Notifications::default(), H).is_empty());
        // Fire, then after reset the entry is pruned and can fire again.
        assert_eq!(t.evaluate(&snap(95.0, H), &Notifications::default(), H).len(), 1);
        t.prune(6 * H);
        assert!(t.fired.is_empty());
    }

    #[test]
    fn persists() {
        let dir = std::env::temp_dir().join(format!("quota-notify-{}", std::process::id()));
        let path = dir.join("notified.json");
        let mut t = Tracker::default();
        t.evaluate(&snap(95.0, H), &Notifications::default(), H);
        t.save(&path).unwrap();
        let mut back = Tracker::load(&path);
        assert_eq!(back, t);
        assert!(back.evaluate(&snap(95.0, H), &Notifications::default(), H).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
