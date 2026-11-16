//! Built-in providers. Each one reads another tool's existing login
//! read-only and calls only that tool's own usage endpoint.

pub mod claude;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod detect;
pub mod openrouter;

use crate::config::Config;
use crate::{Provider, ProviderError};
use serde_json::Value;
use std::path::Path;

pub const ALL_IDS: [&str; 5] = ["claude", "codex", "cursor", "copilot", "openrouter"];

pub fn by_id(id: &str, cfg: &Config) -> Option<Box<dyn Provider>> {
    Some(match id {
        "claude" => Box::new(claude::Claude { max_cache_age_secs: cfg.poll_interval_secs as i64 }),
        "codex" => Box::new(codex::Codex),
        "cursor" => Box::new(cursor::Cursor),
        "copilot" => Box::new(copilot::Copilot),
        "openrouter" => Box::new(openrouter::OpenRouter),
        _ => return None,
    })
}

/// Enabled providers in configured order.
pub fn enabled(cfg: &Config) -> Vec<Box<dyn Provider>> {
    cfg.enabled_providers.iter().filter_map(|id| by_id(id, cfg)).collect()
}

/// Reads a JSON file another tool owns. Missing -> `NotConfigured(missing)`.
pub(crate) fn read_json(path: &Path, missing: &str) -> Result<Value, ProviderError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ProviderError::NotConfigured(missing.into()))
        }
        Err(_) => return Err(ProviderError::Unavailable(format!("cannot read {}", file_name(path)))),
    };
    // Some tools write a UTF-8 BOM.
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    serde_json::from_slice(bytes).map_err(|_| ProviderError::Parse(format!("{} is not valid JSON", file_name(path))))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// OpenCode's data dir: `XDG_DATA_HOME\opencode`, else
/// `%USERPROFILE%\.local\share\opencode` (verified on Windows).
pub(crate) fn opencode_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(|d| std::path::PathBuf::from(d).join("opencode"))
        .or_else(|| crate::paths::home().map(|h| h.join(".local").join("share").join("opencode")))
}

/// OpenCode's `auth.json` (provider logins and API keys), read-only.
pub(crate) fn opencode_auth() -> Option<Value> {
    read_json(&opencode_dir()?.join("auth.json"), "").ok()
}

/// `"pro"` -> `"Pro"`, `"google_play_subscription"` -> `"Google play subscription"`.
pub(crate) fn plan_label(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| crate::json::humanize(s))
}

/// Fails with a clear message if a JWT's `exp` is in the past. Opaque tokens
/// (no readable exp) pass through and are judged by the server.
pub(crate) fn check_jwt_expiry(token: &str, now: i64, tool_hint: &str) -> Result<(), ProviderError> {
    match crate::json::jwt_exp(token) {
        Some(exp) if exp <= now => Err(ProviderError::Auth(format!("token expired; {tool_hint}"))),
        _ => Ok(()),
    }
}
