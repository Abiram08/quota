//! `quota limits list`, `quota refresh` and `quota status`.

use crate::out::{plural, Cell, Ctx, Failure, Table, Warning};
use crate::style::{clean, gutter, Style, Tone};
use quota_core::config::Config;
use quota_core::output::{self, Level};
use quota_core::snapshot::{Snapshot, SnapshotError};
use quota_core::{format, providers, refresh, time, Limit, ProviderError};
use serde_json::{json, Value};
use std::process::ExitCode;

/// A snapshot older than this gets a warning (quota-tray refreshes every
/// few minutes, so this means nothing is keeping it up to date).
pub const OLD_AFTER_SECS: i64 = 15 * 60;
const BAR: usize = 10;

pub fn tone(l: Level) -> Tone {
    match l {
        Level::Ok => Tone::Ok,
        Level::Low => Tone::Low,
        Level::Critical => Tone::Critical,
        Level::None => Tone::Muted,
    }
}

/// Loads the saved snapshot, or the failure that explains why not.
pub fn load() -> Result<Snapshot, Failure> {
    if quota_core::paths::snapshot_file().is_none() {
        return Err(Failure::failed(
            "missing_environment",
            "Can't find Quota's data folder. LOCALAPPDATA is not set.".into(),
            vec![],
        ));
    }
    match Snapshot::load() {
        Ok(s) => Ok(s),
        Err(SnapshotError::Missing) => Err(Failure::action(
            "no_snapshot",
            "Can't list limits. Nothing has been fetched on this PC yet.".into(),
            vec![("To fetch usage now, run:".into(), "quota refresh".into())],
        )),
        Err(e) => Err(Failure::failed(
            "invalid_snapshot",
            format!("Couldn't read the saved snapshot. {}.", output::sentence_case(&e.to_string())),
            vec![("To write a new one, run:".into(), "quota refresh".into())],
        )),
    }
}

/// Warnings about data that may be out of date.
pub fn freshness(snap: &Snapshot, now: i64, after_refresh: bool) -> Vec<Warning> {
    let mut w = Vec::new();
    if !after_refresh && snap.fetched_at > 0 && now - snap.fetched_at > OLD_AFTER_SECS {
        w.push(Warning {
            reason: "snapshot_old",
            message: format!("This data is from {}.", format::relative(snap.fetched_at, now)),
            next: Some(("To update it, run:".into(), "quota refresh".into())),
        });
    }
    for p in snap.providers.iter().filter(|p| p.stale && !p.is_unavailable()) {
        let when = p.fetched_at.map(|t| format::relative(t, now)).unwrap_or_else(|| "earlier".into());
        let why = p.error.as_ref().map(|e| format!(" Its last check failed: {}.", clean(e.message.trim_end_matches('.')))).unwrap_or_default();
        w.push(Warning {
            reason: "stale_data",
            message: format!("{}'s limits are from {when}.{why}", output::short_name(&p.id, &p.name)),
            next: Some(("To try again, run:".into(), format!("quota refresh --provider {}", p.id))),
        });
    }
    w
}

fn bar(l: &Limit, st: Style) -> String {
    let filled = ((l.percent_left() / 100.0).clamp(0.0, 1.0) * BAR as f64).round() as usize;
    let t = tone(Level::for_left(l.percent_left()));
    format!("{}{}", st.tone(t, &"█".repeat(filled)), st.muted(&"░".repeat(BAR - filled)))
}

fn resets(l: &Limit, now: i64) -> String {
    match l.resets_at {
        Some(r) if r <= now => format!("ended {}", format::relative(r, now)),
        Some(r) => format!("in {}", format::duration(r - now)),
        None => "—".into(),
    }
}

pub fn table(snap: &Snapshot, now: i64, st: Style) -> Table {
    let mut t = Table::new(vec!["Provider", "Limit", "Left", "Resets", "Status"], vec![false, false, false, false, false]);
    for p in &snap.providers {
        if p.is_unavailable() {
            let why = p.error.as_ref().map(|e| output::sentence_case(&e.message)).unwrap_or_else(|| "Unavailable".into());
            let status = format!("Not connected: {}", clean(&why));
            t.push(
                vec![Cell::new(&p.id), Cell::new("—"), Cell::new("—"), Cell::new("—"), Cell::new(&status)],
                vec![p.id.clone(), String::new(), String::new(), String::new(), status.clone()],
            );
            continue;
        }
        for l in &p.limits {
            let (status, warn, level) = output::limit_status(l, now);
            let ended = l.has_reset(now);
            let pct = l.percent_left().floor() as i64;
            let left = if ended {
                Cell::new("—")
            } else {
                Cell::styled(format!("{} {:>4}", bar(l, st), format!("{pct}%")))
            };
            let status_cell = if warn { Cell::styled(st.tone_bold(tone(level), &clean(&status))) } else { Cell::new(&status) };
            t.push(
                vec![Cell::new(&p.id), Cell::new(&l.label), left, Cell::new(resets(l, now)), status_cell],
                vec![
                    p.id.clone(),
                    l.label.clone(),
                    if ended { String::new() } else { pct.to_string() },
                    l.resets_at.map(time::format_rfc3339).unwrap_or_default(),
                    status,
                ],
            );
        }
    }
    t
}

/// Terminal view: when it was updated, one summary, then the table.
pub fn render(snap: &Snapshot, now: i64, st: Style) -> String {
    let mut out = String::new();
    let updated = if snap.fetched_at > 0 { format::relative(snap.fetched_at, now) } else { "never".into() };
    out.push_str(&format!("  {:<16}{updated}\n\n", "Updated"));
    let sm = output::summary(snap, now);
    let glyph = if sm.alert || sm.level == Level::Critical { st.warn() } else { String::new() };
    let glyph = if st.color { glyph } else { String::new() };
    out.push_str(&gutter(&glyph, &st.tone_bold(tone(sm.level), &clean(&sm.headline))));
    out.push('\n');
    let mut detail = sm.detail.clone();
    if let Some(h) = &sm.hint {
        detail.push(' ');
        detail.push_str(h);
    }
    out.push_str(&crate::cli::wrap(&clean(&detail), 2));
    out.push_str("\n\n");
    out.push_str(&table(snap, now, st).tty(st));
    out
}

/// The `quota.limits.v2` document, plus `warnings` when there are any.
pub fn doc(snap: &Snapshot, now: i64, warnings: &[Warning]) -> Value {
    let mut v = serde_json::to_value(output::limits_json(snap, now)).unwrap_or(Value::Null);
    if !warnings.is_empty() {
        v["warnings"] = Value::Array(warnings.iter().map(Warning::json).collect());
    }
    v
}

/// Prints the limits of `snap` (already scoped) in the mode `ctx` asks for.
fn show(ctx: &Ctx, snap: &Snapshot, now: i64, filtered: bool, extra: Option<(&str, Value)>, warnings: &[Warning]) -> ExitCode {
    if ctx.json {
        let mut v = doc(snap, now, warnings);
        if let Some((k, x)) = extra {
            v[k] = x;
        }
        return ctx.print_json(&v);
    }
    for w in warnings {
        ctx.warn(w);
    }
    if snap.providers.is_empty() {
        if filtered {
            ctx.note("No limits match this filter.");
            ctx.note(&format!("  To see every provider, run:\n      {}", ctx.err.command("quota limits list")));
        } else {
            ctx.note("No limits");
            ctx.note("  Sign in to Claude Code, Codex, Cursor, GitHub Copilot or OpenRouter, then run quota refresh.");
        }
        if !ctx.out_tty {
            return crate::out::print(&table(snap, now, Style::PLAIN).tsv());
        }
        return ExitCode::SUCCESS;
    }
    if ctx.rich() {
        crate::out::print(&render(snap, now, ctx.out))
    } else {
        crate::out::print(&table(snap, now, Style::PLAIN).tsv())
    }
}

pub fn list(ctx: &Ctx, only: Option<&[String]>) -> ExitCode {
    let snap = match load() {
        Ok(s) => s,
        Err(f) => return f.report(ctx),
    };
    let now = time::now();
    let scoped = output::scoped(&snap, only);
    let warnings = freshness(&scoped, now, false);
    show(ctx, &scoped, now, only.is_some(), None, &warnings)
}

/// Not signed in, or a lasting reason the provider can't be read (an
/// encrypted token, no plan): skipped, so it doesn't fail every refresh.
/// Network, sign-in and response errors are failures worth an exit 1.
fn skipped(e: &ProviderError) -> bool {
    matches!(e, ProviderError::NotConfigured(_) | ProviderError::Unavailable(_))
}

/// The outcome of one refresh, for the CLI receipt and the MCP tool.
pub struct Refreshed {
    pub snap: Snapshot,
    pub now: i64,
    pub took: std::time::Duration,
    /// `{provider, outcome: refreshed|skipped|failed, error?}`
    pub rows: Vec<Value>,
    pub ok: usize,
    pub skipped: usize,
    /// `(name, reason)`
    pub failed: Vec<(String, String)>,
    pub warnings: Vec<Warning>,
}

impl Refreshed {
    /// The bulk part of the JSON result (`refresh`).
    pub fn json(&self) -> Value {
        json!({
            "results": self.rows,
            "summary": {"succeeded": self.ok, "failed": self.failed.len(), "skipped": self.skipped},
        })
    }
}

/// Fetches the given providers (None = the enabled ones) in parallel,
/// folds the results into the saved snapshot and saves it. `started` is
/// called with the provider count just before the network work begins.
pub fn refresh_now(only: Option<&[String]>, started: impl FnOnce(usize)) -> Result<Refreshed, Failure> {
    let (cfg, cfg_warning) = Config::load();
    let mut warnings: Vec<Warning> = cfg_warning
        .map(|m| Warning { reason: "invalid_config", message: format!("{}.", m.trim_end_matches('.')), next: None })
        .into_iter()
        .collect();
    if quota_core::paths::snapshot_file().is_none() {
        return Err(Failure::failed("missing_environment", "Can't save usage. LOCALAPPDATA is not set.".into(), vec![]));
    }
    let mut order = cfg.enabled_providers.clone();
    let list: Vec<Box<dyn quota_core::Provider>> = match only {
        Some(ids) => {
            for id in ids {
                if !order.contains(id) {
                    order.push(id.clone());
                }
            }
            ids.iter().filter_map(|id| providers::by_id(id, &cfg)).collect()
        }
        None => providers::enabled(&cfg),
    };
    if list.is_empty() {
        return Err(Failure::action(
            "no_providers",
            "Can't refresh. Every provider is turned off in the config file.".into(),
            vec![("To find the config file, run:".into(), "quota config path".into())],
        ));
    }
    started(list.len());
    let t0 = std::time::Instant::now();
    let refs: Vec<&dyn quota_core::Provider> = list.iter().map(|b| b.as_ref()).collect();
    let results = refresh::fetch_all(&refs);
    let took = t0.elapsed();

    let mut rows: Vec<Value> = Vec::new();
    let (mut ok, mut failed, mut skip) = (0usize, Vec::new(), 0usize);
    for (id, name, r) in &results {
        match r {
            Ok(_) => {
                ok += 1;
                rows.push(json!({"provider": id, "outcome": "refreshed"}));
            }
            Err(e) if skipped(e) => {
                skip += 1;
                rows.push(json!({"provider": id, "outcome": "skipped", "error": {"reason": e.kind(), "message": e.message()}}));
            }
            Err(e) => {
                failed.push((output::short_name(id, name), output::sentence_case(e.message())));
                rows.push(json!({"provider": id, "outcome": "failed", "error": {"reason": e.kind(), "message": e.message()}}));
            }
        }
    }
    let now = time::now();
    let mut snap = Snapshot::load().unwrap_or_default();
    refresh::apply(&mut snap, results, now);
    snap.retain_order(&order);
    if let Err(e) = snap.save() {
        warnings.push(Warning {
            reason: "snapshot_not_saved",
            message: format!("Couldn't save the snapshot: {e}. These limits are not kept."),
            next: None,
        });
    }
    Ok(Refreshed { snap, now, took, rows, ok, skipped: skip, failed, warnings })
}

pub fn run_refresh(ctx: &Ctx, only: Option<&[String]>) -> ExitCode {
    let mut progress = String::new();
    let r = refresh_now(only, |n| {
        progress = format!("Refreshing {}…", plural(n, "provider", "providers"));
        ctx.progress(&progress);
    });
    ctx.progress_done(&progress);
    let r = match r {
        Ok(r) => r,
        Err(f) => return f.report(ctx),
    };
    let Refreshed { ref snap, now, took, ok, skipped: skip, ref failed, ref warnings, .. } = r;

    // Receipt (stderr), then the limits (stdout).
    let secs = format!("{:.1}s", took.as_secs_f64());
    let skipped_note =
        if skip > 0 { format!(" Skipped {} that can't be read.", plural(skip, "provider", "providers")) } else { String::new() };
    if failed.is_empty() {
        ctx.receipt(&format!("Refreshed {} in {secs}.{skipped_note}", plural(ok, "provider", "providers")));
    } else if !ctx.json {
        let total = ok + failed.len();
        eprintln!(
            "{}",
            gutter(&ctx.err.fail(), &format!("Refreshed {ok} of {total} providers. {} failed.{skipped_note}", failed.len()))
        );
        let w = failed.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0);
        for (name, why) in failed {
            eprintln!("  {name:<w$}  {}", clean(why));
        }
    }
    let shown = output::scoped(snap, only);
    let code = show(ctx, &shown, now, only.is_some(), Some(("refresh", r.json())), warnings);
    if failed.is_empty() {
        code
    } else {
        ExitCode::FAILURE
    }
}

/// `Claude Session (5h): 50% left, out in 1h 0m`, plus how old it is when
/// that matters.
pub fn status_line(snap: &Snapshot, now: i64) -> (String, Level, Option<(String, String)>) {
    let old = snap.fetched_at > 0 && now - snap.fetched_at > OLD_AFTER_SECS;
    let age = if old { format!(" (from {})", format::relative(snap.fetched_at, now)) } else { String::new() };
    match output::most_urgent(snap, now, 1).into_iter().next() {
        Some((name, l)) => {
            let level = output::limit_status(l, now).2;
            (format!("{}{age}", clean(&output::summary_line(&name, l, now))), level, Some((l.provider.clone(), l.label.clone())))
        }
        None => (format!("No limits{age}"), Level::None, None),
    }
}

pub fn status(ctx: &Ctx, only: Option<&[String]>) -> ExitCode {
    let snap = match load() {
        Ok(s) => output::scoped(&s, only),
        Err(f) => return f.report(ctx),
    };
    let now = time::now();
    let (line, level, which) = status_line(&snap, now);
    if ctx.json {
        return ctx.print_json(&json!({
            "schema": "quota.status.v1",
            "line": line,
            "level": output::level_str(level),
            "provider": which.as_ref().map(|w| &w.0),
            "limit": which.as_ref().map(|w| &w.1),
            "updatedAt": (snap.fetched_at > 0).then(|| time::format_rfc3339(snap.fetched_at)),
        }));
    }
    crate::out::print(&ctx.out.tone(tone(level), &line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quota_core::Unit;

    const H: i64 = 3600;

    fn lim(label: &str, used: f64, start: i64, end: i64) -> Limit {
        Limit {
            provider: "claude".into(),
            account_label: Some("Pro".into()),
            label: label.into(),
            used,
            limit: 100.0,
            unit: Unit::Percent,
            window_start: Some(start),
            resets_at: Some(end),
        }
    }

    fn sample() -> Snapshot {
        let mut s = Snapshot::default();
        s.apply(
            "claude",
            "Claude Code",
            Ok(vec![lim("Session (5h)", 50.0, 0, 5 * H), lim("Weekly", 10.0, 0, 7 * 24 * H)]),
            H,
        );
        s.apply("cursor", "Cursor", Err(ProviderError::Unavailable("sign-in token is encrypted".into())), H);
        s
    }

    #[test]
    fn terminal_view_has_context_summary_and_table() {
        let t = render(&sample(), H, Style::PLAIN);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "  Updated         now", "label in a 16-column field from column 2");
        assert_eq!(lines[2], "  Claude runs out in 1h 0m");
        assert!(lines[3].starts_with("  50% of Session (5h) left"), "{t}");
        assert!(lines.iter().all(|l| l.chars().count() <= 80) || t.contains("Not connected"), "{t}");
        let head = lines.iter().position(|l| l.starts_with("Provider  Limit")).expect("table header");
        assert!(lines[head - 1].is_empty() && lines[head].contains("Status"), "{t}");
        let row = lines[head + 1];
        assert!(row.starts_with("claude    Session (5h)  █████░░░░░  50%  in 4h 0m"), "{t}");
        assert!(row.ends_with("Out in 1h 0m"), "{t}");
        assert!(t.contains("cursor    —"), "{t}");
        assert!(t.contains("Not connected: Sign-in token is encrypted"), "{t}");
        assert!(t.lines().all(|l| l == l.trim_end()), "no trailing spaces");
    }

    #[test]
    fn piped_view_is_tab_separated_with_iso_times() {
        let t = table(&sample(), H, Style::PLAIN).tsv();
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "Provider\tLimit\tLeft\tResets\tStatus");
        assert_eq!(lines[1], "claude\tSession (5h)\t50\t1970-01-01T05:00:00Z\tOut in 1h 0m");
        assert_eq!(lines[3], "cursor\t\t\t\tNot connected: Sign-in token is encrypted");
        // After the session window ended: no stale percentage.
        let t = table(&sample(), 5 * H + 60, Style::PLAIN).tsv();
        assert!(t.lines().nth(1).unwrap().starts_with("claude\tSession (5h)\t\t"), "{t}");
    }

    #[test]
    fn colour_keeps_alignment() {
        let plain = render(&sample(), H, Style::PLAIN);
        let colour = render(&sample(), H, Style { color: true });
        assert!(colour.contains("\x1b["));
        let w = |s: &str| s.lines().skip(2).map(crate::style::width).collect::<Vec<_>>();
        assert_eq!(w(&plain)[1..], w(&colour)[1..], "only the glyph column may differ");
    }

    #[test]
    fn status_line_and_age() {
        let (line, level, which) = status_line(&sample(), H);
        assert_eq!(line, "Claude Session (5h): 50% left, out in 1h 0m");
        assert_eq!(level, Level::Low);
        assert_eq!(which.unwrap().1, "Session (5h)");
        let (old, _, _) = status_line(&sample(), H + OLD_AFTER_SECS + 60);
        assert!(old.ends_with("(from 16m ago)"), "{old}");
        assert_eq!(status_line(&Snapshot::default(), 0).0, "No limits");
    }

    #[test]
    fn freshness_warnings() {
        let s = sample();
        assert!(freshness(&s, H, false).is_empty());
        let w = freshness(&s, H + OLD_AFTER_SECS + 1, false);
        assert_eq!(w[0].reason, "snapshot_old");
        assert!(freshness(&s, H + OLD_AFTER_SECS + 1, true).is_empty(), "not right after a refresh");
    }
}
