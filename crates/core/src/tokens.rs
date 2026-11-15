//! Token usage from the session logs coding tools keep on this PC.
//!
//! Verified sources (read-only, numbers and dates only, never content):
//! * Claude Code: `~/.claude/projects/**/*.jsonl` (`CLAUDE_CONFIG_DIR`
//!   overrides `~/.claude`). Assistant lines carry `timestamp` and
//!   `message.usage.{input_tokens, output_tokens,
//!   cache_creation_input_tokens, cache_read_input_tokens}`; streaming can
//!   repeat a line for the same `message.id`, which is counted once.
//! * Codex: `~/.codex/sessions/**/*.jsonl` (`CODEX_HOME` overrides
//!   `~/.codex`). `event_msg` / `token_count` lines carry `timestamp` and a
//!   running `info.total_token_usage.total_tokens`; the increase since the
//!   previous event is attributed to that event's day.
//!
//! Both counts include cached input tokens. Logs are append-only, so the
//! scan is incremental: a cache in the data dir remembers each file's size,
//! the byte offset already read and per-day totals. Unchanged files are
//! skipped; grown files are read from the saved offset; shrunk files are
//! re-read from the start.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const CACHE_VERSION: u32 = 1;
/// Per-day history kept in the cache.
const KEEP_DAYS: i64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Claude,
    Codex,
}

impl Source {
    pub fn id(self) -> &'static str {
        match self {
            Source::Claude => "claude",
            Source::Codex => "codex",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Source::Claude => "Claude",
            Source::Codex => "Codex",
        }
    }
}

/// Scan state for one log file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct FileState {
    size: u64,
    offset: u64,
    /// Claude: last counted message id (streaming repeats are skipped).
    #[serde(default)]
    last_id: String,
    /// Codex: last running total seen.
    #[serde(default)]
    last_total: u64,
    /// Local day number (days since 1970-01-01) -> tokens.
    #[serde(default)]
    days: BTreeMap<i64, u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Cache {
    version: u32,
    /// path -> (source, state)
    files: BTreeMap<String, (Source, FileState)>,
}

/// Daily totals for one source.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SourceTokens {
    pub days: BTreeMap<i64, u64>,
    pub files: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub claude: SourceTokens,
    pub codex: SourceTokens,
}

impl Report {
    pub fn get(&self, s: Source) -> &SourceTokens {
        match s {
            Source::Claude => &self.claude,
            Source::Codex => &self.codex,
        }
    }

    /// Sum over local days `from..=to`.
    pub fn sum(&self, s: Source, from: i64, to: i64) -> u64 {
        self.get(s).days.range(from..=to).map(|(_, v)| *v).sum()
    }

    /// `n` daily totals ending at `today`, oldest first.
    pub fn series(&self, s: Source, today: i64, n: i64) -> Vec<u64> {
        (today - n + 1..=today).map(|d| self.get(s).days.get(&d).copied().unwrap_or(0)).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.claude.files == 0 && self.codex.files == 0
    }
}

/// Local calendar day number for a unix timestamp.
pub fn local_day(ts: i64) -> i64 {
    let c = crate::time::civil_local(ts);
    crate::time::days_from_civil(c.year, c.month, c.day)
}

fn claude_root() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::paths::home().map(|h| h.join(".claude")))
        .map(|d| d.join("projects"))
}

fn codex_root() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::paths::home().map(|h| h.join(".codex")))
        .map(|d| d.join("sessions"))
}

/// All `*.jsonl` files under `root` (bounded depth).
fn jsonl_files(root: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() && depth < 6 => jsonl_files(&p, out, depth + 1),
            Ok(t) if t.is_file() && p.extension().is_some_and(|x| x == "jsonl") => out.push(p),
            _ => {}
        }
    }
}

/// Claude: (timestamp, message id, tokens) from one log line.
pub fn parse_claude_line(line: &str) -> Option<(i64, String, u64)> {
    if !line.contains("\"usage\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let msg = v.get("message")?;
    let u = msg.get("usage")?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let tokens = n("input_tokens") + n("output_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
    let ts = crate::time::parse_rfc3339(v.get("timestamp")?.as_str()?)?;
    let id = msg.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    Some((ts, id, tokens))
}

/// Codex: (timestamp, running total tokens) from one log line.
pub fn parse_codex_line(line: &str) -> Option<(i64, u64)> {
    if !line.contains("\"token_count\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let total = v
        .get("payload")?
        .get("info")?
        .get("total_token_usage")?
        .get("total_tokens")?
        .as_u64()?;
    let ts = crate::time::parse_rfc3339(v.get("timestamp")?.as_str()?)?;
    Some((ts, total))
}

/// Reads `path` from the saved offset, updating `st`. Only complete lines
/// are consumed, so a line being written right now is read next time.
fn scan_file(path: &Path, source: Source, st: &mut FileState) -> std::io::Result<()> {
    let size = std::fs::metadata(path)?.len();
    if size < st.offset {
        *st = FileState::default(); // rewritten: start over
    }
    if size == st.offset {
        st.size = size;
        return Ok(());
    }
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(st.offset))?;
    let mut r = BufReader::with_capacity(64 * 1024, f);
    let mut line = String::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line)?;
        if n == 0 || !line.ends_with('\n') {
            break; // EOF or a partial last line
        }
        st.offset += n as u64;
        match source {
            Source::Claude => {
                if let Some((ts, id, tokens)) = parse_claude_line(&line) {
                    if id.is_empty() || id != st.last_id {
                        *st.days.entry(local_day(ts)).or_default() += tokens;
                        st.last_id = id;
                    }
                }
            }
            Source::Codex => {
                if let Some((ts, total)) = parse_codex_line(&line) {
                    // The running total restarts with a new session.
                    let delta = if total >= st.last_total { total - st.last_total } else { total };
                    if delta > 0 {
                        *st.days.entry(local_day(ts)).or_default() += delta;
                    }
                    st.last_total = total;
                }
            }
        }
    }
    st.size = size;
    Ok(())
}

fn load_cache(path: &Path) -> Cache {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Cache>(&b).ok())
        .filter(|c| c.version == CACHE_VERSION)
        .unwrap_or(Cache { version: CACHE_VERSION, files: BTreeMap::new() })
}

fn save_cache(path: &Path, c: &Cache) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(bytes) = serde_json::to_vec(c) {
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// Scans the given roots with the cache at `cache_path` and returns the
/// report. Used directly by tests; `scan()` supplies the real paths.
pub fn scan_with(claude: Option<&Path>, codex: Option<&Path>, cache_path: &Path, now: i64) -> Report {
    let mut cache = load_cache(cache_path);
    let mut seen: Vec<String> = Vec::new();
    for (root, source) in [(claude, Source::Claude), (codex, Source::Codex)] {
        let Some(root) = root else { continue };
        let mut files = Vec::new();
        jsonl_files(root, &mut files, 0);
        for p in files {
            let key = p.to_string_lossy().into_owned();
            let entry = cache.files.entry(key.clone()).or_insert((source, FileState::default()));
            let _ = scan_file(&p, source, &mut entry.1);
            seen.push(key);
        }
    }
    // Forget deleted files and old days.
    cache.files.retain(|k, _| seen.contains(k));
    let oldest = local_day(now) - KEEP_DAYS;
    for (_, st) in cache.files.values_mut() {
        st.days.retain(|d, _| *d >= oldest);
    }
    save_cache(cache_path, &cache);

    let mut report = Report::default();
    for (source, st) in cache.files.values() {
        let t = match source {
            Source::Claude => &mut report.claude,
            Source::Codex => &mut report.codex,
        };
        t.files += 1;
        for (d, v) in &st.days {
            *t.days.entry(*d).or_default() += v;
        }
    }
    report
}

/// Scans this PC's Claude Code and Codex logs (incremental, cached).
pub fn scan(now: i64) -> Report {
    let Some(cache) = crate::paths::data_dir().map(|d| d.join("tokens-cache.json")) else {
        return Report::default();
    };
    scan_with(claude_root().as_deref(), codex_root().as_deref(), &cache, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_claude_usage_lines() {
        let line = r#"{"type":"assistant","timestamp":"2026-11-12T10:00:00Z","message":{"id":"msg_1","content":[{"type":"text","text":"secret"}],"usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000}}}"#;
        let (ts, id, t) = parse_claude_line(line).unwrap();
        assert_eq!((id.as_str(), t), ("msg_1", 1115));
        assert_eq!(ts, crate::time::parse_rfc3339("2026-11-12T10:00:00Z").unwrap());
        assert!(parse_claude_line(r#"{"type":"user","message":{"content":"hi"}}"#).is_none());
        assert!(parse_claude_line("not json \"usage\"").is_none());
    }

    #[test]
    fn parses_codex_token_counts() {
        let line = r#"{"timestamp":"2026-11-12T10:00:00.123Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":800,"cached_input_tokens":500,"output_tokens":200,"total_tokens":1000},"last_token_usage":{"total_tokens":300}}}}"#;
        assert_eq!(parse_codex_line(line).map(|x| x.1), Some(1000));
        // Rate-limit-only event without info.
        assert!(parse_codex_line(r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#).is_none());
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("quota-tokens-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn append(path: &Path, lines: &[&str]) {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    #[test]
    fn incremental_scan_counts_each_line_once() {
        let root = tmp("inc");
        let (claude, codex) = (root.join("claude"), root.join("codex"));
        std::fs::create_dir_all(claude.join("proj")).unwrap();
        std::fs::create_dir_all(codex.join("2026/11/12")).unwrap();
        let cache = root.join("cache.json");
        let now = crate::time::parse_rfc3339("2026-11-12T12:00:00Z").unwrap();
        let day = local_day(crate::time::parse_rfc3339("2026-11-12T10:00:00Z").unwrap());
        let c = |id: &str, n: u64| {
            format!(r#"{{"timestamp":"2026-11-12T10:00:00Z","message":{{"id":"{id}","usage":{{"input_tokens":{n},"output_tokens":0}}}}}}"#)
        };
        let x = |total: u64| {
            format!(r#"{{"timestamp":"2026-11-12T10:00:00Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"total_tokens":{total}}}}}}}}}"#)
        };
        let cf = claude.join("proj/s.jsonl");
        let xf = codex.join("2026/11/12/rollout.jsonl");
        // A streaming repeat (same id) counts once.
        append(&cf, &[&c("a", 100), &c("a", 100), &c("b", 50)]);
        append(&xf, &[&x(1000), &x(1000), &x(1500)]);
        let r = scan_with(Some(&claude), Some(&codex), &cache, now);
        assert_eq!(r.sum(Source::Claude, day, day), 150);
        assert_eq!(r.sum(Source::Codex, day, day), 1500);
        assert_eq!((r.claude.files, r.codex.files), (1, 1));

        // Appending: only new lines are read; earlier counts are not doubled.
        append(&cf, &[&c("c", 25)]);
        append(&xf, &[&x(1600)]);
        let r = scan_with(Some(&claude), Some(&codex), &cache, now);
        assert_eq!(r.sum(Source::Claude, day, day), 175);
        assert_eq!(r.sum(Source::Codex, day, day), 1600);

        // A partial last line is left for the next scan.
        let mut f = std::fs::OpenOptions::new().append(true).open(&cf).unwrap();
        write!(f, "{}", c("d", 5)).unwrap();
        drop(f);
        assert_eq!(scan_with(Some(&claude), Some(&codex), &cache, now).sum(Source::Claude, day, day), 175);
        append(&cf, &[""]);
        assert_eq!(scan_with(Some(&claude), Some(&codex), &cache, now).sum(Source::Claude, day, day), 180);

        // A rewritten (shrunk) file is re-read from the start.
        std::fs::write(&cf, format!("{}\n", c("z", 7))).unwrap();
        assert_eq!(scan_with(Some(&claude), Some(&codex), &cache, now).sum(Source::Claude, day, day), 7);

        // A deleted file is forgotten.
        std::fs::remove_file(&xf).unwrap();
        let r = scan_with(Some(&claude), Some(&codex), &cache, now);
        assert_eq!((r.codex.files, r.sum(Source::Codex, day, day)), (0, 0));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn series_and_sums() {
        let mut r = Report::default();
        r.claude.days.insert(10, 5);
        r.claude.days.insert(12, 7);
        assert_eq!(r.series(Source::Claude, 12, 4), vec![0, 5, 0, 7]);
        assert_eq!(r.sum(Source::Claude, 11, 12), 7);
        assert!(r.is_empty(), "no files scanned");
    }
}
