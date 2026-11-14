//! `%APPDATA%\quota\config.toml`. Every field is optional; a missing or
//! broken file yields defaults so the app always starts.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MIN_POLL_SECS: u64 = 60;
pub const DEFAULT_POLL_SECS: u64 = 180;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Seconds between polls (minimum 60).
    pub poll_interval_secs: u64,
    /// Provider ids to fetch, in display order.
    pub enabled_providers: Vec<String>,
    /// Popup colours: follow Windows, or force light / dark.
    pub theme: Theme,
    /// Starred metrics, always shown under Pinned: `"provider/Label"`,
    /// e.g. `"claude/Weekly"`.
    pub pinned: Vec<String>,
    /// Bars and figures show what is left, or what is used.
    pub usage_mode: UsageMode,
    /// Reset times as a countdown ("Resets in 4h 2m") or a clock time.
    pub reset_timers: ResetTimers,
    pub notifications: Notifications,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageMode {
    #[default]
    Left,
    Used,
}

impl UsageMode {
    pub fn as_str(self) -> &'static str {
        match self {
            UsageMode::Left => "left",
            UsageMode::Used => "used",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResetTimers {
    #[default]
    Relative,
    Absolute,
}

impl ResetTimers {
    pub fn as_str(self) -> &'static str {
        match self {
            ResetTimers::Relative => "relative",
            ResetTimers::Absolute => "absolute",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    /// Title-case label for menus and the popup.
    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "System",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }

    /// System -> Light -> Dark -> System.
    pub fn next(self) -> Theme {
        match self {
            Theme::System => Theme::Light,
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::System,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub enabled: bool,
    /// Notify when percent *left* drops to or below each value.
    pub thresholds_percent_left: Vec<f64>,
    /// Notify when pace says a limit will run out before it resets.
    pub pace_warnings: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            poll_interval_secs: DEFAULT_POLL_SECS,
            enabled_providers: crate::providers::ALL_IDS.iter().map(|s| s.to_string()).collect(),
            theme: Theme::System,
            pinned: Vec::new(),
            usage_mode: UsageMode::Left,
            reset_timers: ResetTimers::Relative,
            notifications: Notifications::default(),
        }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Notifications { enabled: true, thresholds_percent_left: vec![30.0, 10.0], pace_warnings: true }
    }
}

impl Config {
    /// Loads from the default path; any problem returns defaults plus a
    /// warning string the caller may surface.
    pub fn load() -> (Config, Option<String>) {
        let Some(path) = crate::paths::config_file() else {
            return (Config::default(), Some("APPDATA is not set; using defaults".into()));
        };
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> (Config, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match Self::parse(&text) {
                Ok(c) => (c, None),
                Err(e) => (Config::default(), Some(format!("config.toml ignored: {e}"))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
            Err(e) => (Config::default(), Some(format!("config.toml unreadable: {e}"))),
        }
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        let mut c: Config = basic_toml::from_str(text).map_err(|e| e.to_string())?;
        c.normalize();
        Ok(c)
    }

    fn normalize(&mut self) {
        self.poll_interval_secs = self.poll_interval_secs.max(MIN_POLL_SECS);
        self.enabled_providers = self
            .enabled_providers
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| crate::providers::ALL_IDS.contains(&s.as_str()))
            .fold(Vec::new(), |mut acc, s| {
                if !acc.contains(&s) {
                    acc.push(s);
                }
                acc
            });
        self.pinned.retain(|k| k.split_once('/').is_some_and(|(p, l)| !p.trim().is_empty() && !l.trim().is_empty()));
        self.pinned.dedup();
        let t = &mut self.notifications.thresholds_percent_left;
        t.retain(|v| v.is_finite() && *v > 0.0 && *v < 100.0);
        t.sort_by(|a, b| b.total_cmp(a));
        t.dedup();
    }

    /// Saves `theme` into config.toml, keeping every other line (and the
    /// user's comments) as they are. Creates the default file first.
    pub fn save_theme(theme: Theme) -> std::io::Result<()> {
        let path = Self::write_default_if_missing()?;
        let text = std::fs::read_to_string(&path)?;
        std::fs::write(&path, with_theme(&text, theme))
    }

    /// Saves one key (see [`set_value`]) into config.toml in place.
    /// Creates the default file first.
    pub fn save_value(table: Option<&str>, key: &str, value: &str) -> std::io::Result<()> {
        let path = Self::write_default_if_missing()?;
        let text = std::fs::read_to_string(&path)?;
        let out = set_value(&text, table, key, value);
        // Never write a file Quota itself could not read back.
        Self::parse(&out).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, out)
    }

    /// Writes a commented default config if none exists. Returns the path.
    pub fn write_default_if_missing() -> std::io::Result<std::path::PathBuf> {
        let path = crate::paths::config_file()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "APPDATA is not set"))?;
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, DEFAULT_TOML)?;
        }
        Ok(path)
    }
}

/// Replaces the top-level `theme = ...` line, or inserts one before the
/// first table. Text-based so comments and ordering survive.
pub fn with_theme(text: &str, theme: Theme) -> String {
    set_value(text, None, "theme", &format!("\"{}\"", theme.as_str()))
}

/// True if line `t` (trimmed) assigns `key`.
fn assigns(t: &str, key: &str) -> bool {
    t.strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Sets `key = value` (value already in TOML syntax) at the top level
/// (`table` None) or inside `[table]`, keeping every other line and the
/// user's comments. Replaces the first existing assignment and drops any
/// duplicates; otherwise inserts it at the end of that section (before the
/// first table for top-level keys), creating `[table]` if needed.
pub fn set_value(text: &str, table: Option<&str>, key: &str, value: &str) -> String {
    let line = format!("{key} = {value}");
    let header = table.map(|t| format!("[{t}]"));
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    // The section the current line is in: None = top level.
    let mut section: Option<String> = None;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            // Leaving the target section without having written the key.
            if !done && section.as_deref() == header.as_deref() {
                while out.last().is_some_and(|x| x.trim().is_empty()) {
                    out.pop();
                }
                out.push(line.clone());
                out.push(String::new());
                done = true;
            }
            section = Some(t.split('#').next().unwrap_or(t).trim().to_string());
        }
        if section.as_deref() == header.as_deref() && assigns(t, key) {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !done {
        if section.as_deref() != header.as_deref() {
            // The table does not exist yet.
            if let Some(h) = &header {
                if out.last().is_some_and(|x| !x.trim().is_empty()) {
                    out.push(String::new());
                }
                out.push(h.clone());
            }
        }
        out.push(line);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// `["claude", "codex"]`
pub fn toml_string_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|i| format!("\"{}\"", i.replace(['"', '\\'], ""))).collect();
    format!("[{}]", quoted.join(", "))
}

pub const DEFAULT_TOML: &str = r#"# Quota configuration
# Seconds between usage polls (minimum 60). Failing providers back off
# exponentially up to 30 minutes.
poll_interval_secs = 180

# Providers to show, in order: claude, codex, cursor, copilot, openrouter
enabled_providers = ["claude", "codex", "cursor", "copilot", "openrouter"]

# Popup colours: "system" follows Windows, or "light" / "dark".
theme = "system"

# Starred metrics, always shown under Pinned ("provider/Label").
pinned = []

# Show what is "left" of each limit, or what is "used".
usage_mode = "left"

# Reset times as a countdown ("relative") or a clock time ("absolute").
reset_timers = "relative"

[notifications]
enabled = true
# Notify when a limit has this many percent LEFT (or less).
thresholds_percent_left = [30, 10]
# Notify when the current pace would exhaust a limit before it resets.
pace_warnings = true
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toml_matches_defaults() {
        assert_eq!(Config::parse(DEFAULT_TOML).unwrap(), Config::default());
    }

    #[test]
    fn display_modes() {
        let c = Config::parse("usage_mode = \"used\"\nreset_timers = \"absolute\"").unwrap();
        assert_eq!((c.usage_mode, c.reset_timers), (UsageMode::Used, ResetTimers::Absolute));
        assert_eq!(Config::parse("").unwrap().usage_mode, UsageMode::Left);
        assert!(Config::parse("usage_mode = \"half\"").is_err());
        let out = set_value(DEFAULT_TOML, None, "usage_mode", "\"used\"");
        assert_eq!(Config::parse(&out).unwrap().usage_mode, UsageMode::Used);
        assert!(out.contains("# Show what is"));
    }

    #[test]
    fn partial_and_normalized() {
        let c = Config::parse(
            "poll_interval_secs = 5\nenabled_providers = [\"Codex\", \"bogus\", \"codex\"]\n[notifications]\nthresholds_percent_left = [5, 50, 500, 50]\n",
        )
        .unwrap();
        assert_eq!(c.poll_interval_secs, MIN_POLL_SECS);
        assert_eq!(c.enabled_providers, vec!["codex"]);
        assert_eq!(c.notifications.thresholds_percent_left, vec![50.0, 5.0]);
        assert!(c.notifications.enabled);
    }

    #[test]
    fn theme_parses_and_saves_in_place() {
        assert_eq!(Config::parse("theme = \"dark\"").unwrap().theme, Theme::Dark);
        assert!(Config::parse("theme = \"purple\"").is_err());
        assert_eq!(Theme::System.next().next().next(), Theme::System);
        // Replaces the existing line and keeps comments.
        let out = with_theme(DEFAULT_TOML, Theme::Light);
        assert_eq!(Config::parse(&out).unwrap().theme, Theme::Light);
        assert!(out.contains("# Popup colours"));
        assert_eq!(out.matches("theme =").count(), 1);
        // Inserts before the first table when missing (never inside it).
        let out = with_theme("poll_interval_secs = 120\n[notifications]\nenabled = false\n", Theme::Dark);
        let c = Config::parse(&out).unwrap();
        assert_eq!((c.theme, c.poll_interval_secs, c.notifications.enabled), (Theme::Dark, 120, false));
        // A `theme` key inside a table is left alone.
        let out = with_theme("[x]\ntheme = 1\n", Theme::Dark);
        assert!(out.starts_with("theme = \"dark\""));
        assert!(out.contains("[x]\ntheme = 1"));
    }

    #[test]
    fn set_value_in_place() {
        // Top level: replaced, comments kept.
        let out = set_value(DEFAULT_TOML, None, "poll_interval_secs", "600");
        assert_eq!(Config::parse(&out).unwrap().poll_interval_secs, 600);
        assert!(out.contains("# Seconds between usage polls"));
        // Inside a table, never the top-level key of the same name.
        let out = set_value(DEFAULT_TOML, Some("notifications"), "enabled", "false");
        let c = Config::parse(&out).unwrap();
        assert!(!c.notifications.enabled);
        assert!(c.notifications.pace_warnings);
        assert_eq!(out.matches("enabled = ").count(), 1);
        // Missing key in an existing table: appended to that table.
        let out = set_value("[notifications]\npace_warnings = false\n\n[x]\n", Some("notifications"), "enabled", "false");
        assert!(out.contains("[notifications]\npace_warnings = false\nenabled = false\n\n[x]"), "{out}");
        // Missing table: created at the end.
        let out = set_value("theme = \"dark\"\n", Some("notifications"), "enabled", "false");
        assert_eq!(out, "theme = \"dark\"\n\n[notifications]\nenabled = false\n");
        // Duplicates collapse to one.
        let out = set_value("pinned = []\npinned = [\"a/b\"]\n", None, "pinned", "[]");
        assert_eq!(out, "pinned = []\n");
        // Lists.
        let list = toml_string_list(&["claude".into(), "codex".into()]);
        let out = set_value(DEFAULT_TOML, None, "enabled_providers", &list);
        assert_eq!(Config::parse(&out).unwrap().enabled_providers, vec!["claude", "codex"]);
        // A key that only shares a prefix is left alone.
        let out = set_value("themes = 1\n", None, "theme", "\"dark\"");
        assert!(out.contains("themes = 1") && out.contains("theme = \"dark\""));
    }

    #[test]
    fn empty_and_broken() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert!(Config::parse("poll_interval_secs = \"x\"").is_err());
        let dir = std::env::temp_dir().join(format!("quota-cfg-{}", std::process::id()));
        let (c, warn) = Config::load_from(&dir.join("missing.toml"));
        assert_eq!(c, Config::default());
        assert!(warn.is_none());
    }
}
