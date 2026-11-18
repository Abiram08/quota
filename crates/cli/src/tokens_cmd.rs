//! `quota tokens list`: tokens used, from the Claude Code and Codex session
//! logs on this PC (incremental scan, see quota_core::tokens).

use crate::out::{commas, Cell, Ctx, Table};
use crate::style::{Style, Tone};
use quota_core::format;
use quota_core::tokens::{self, Report, Source};
use std::process::ExitCode;

pub const SOURCES: [Source; 2] = [Source::Claude, Source::Codex];
const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// `▁▂▅█▃` scaled to the series maximum. Zero days are the lowest step,
/// drawn muted by the caller.
pub fn sparkline(values: &[u64]) -> String {
    let max = values.iter().copied().max().unwrap_or(0);
    values
        .iter()
        .map(|&v| {
            if max == 0 || v == 0 {
                SPARK[0]
            } else {
                SPARK[((v as f64 / max as f64) * 7.0).round() as usize].max(SPARK[1])
            }
        })
        .collect()
}

struct Totals {
    today: u64,
    yesterday: u64,
    month: u64,
}

fn totals(r: &Report, s: Source, today: i64) -> Totals {
    Totals { today: r.sum(s, today, today), yesterday: r.sum(s, today - 1, today - 1), month: r.sum(s, today - 29, today) }
}

pub fn table(r: &Report, now: i64, st: Style) -> Table {
    let today = tokens::local_day(now);
    let all: u64 = SOURCES.iter().map(|s| totals(r, *s, today).month).sum();
    let mut t = Table::new(
        vec!["Tool", "Today", "Yesterday", "30 days", "Share", "Last 14 days"],
        vec![false, true, true, true, true, false],
    );
    for s in SOURCES {
        let x = totals(r, s, today);
        let share = if all > 0 { (x.month as f64 / all as f64 * 100.0).round() as i64 } else { 0 };
        let series = r.series(s, today, 14);
        let spark: String = sparkline(&series)
            .chars()
            .zip(series.iter())
            .map(|(c, v)| if *v == 0 { st.muted(&c.to_string()) } else { st.tone(Tone::Ok, &c.to_string()) })
            .collect();
        t.push(
            vec![
                Cell::new(s.id()),
                Cell::new(commas(x.today)),
                Cell::new(commas(x.yesterday)),
                Cell::new(commas(x.month)),
                Cell::new(format!("{share}%")),
                Cell::styled(spark),
            ],
            vec![
                s.id().into(),
                x.today.to_string(),
                x.yesterday.to_string(),
                x.month.to_string(),
                share.to_string(),
                series.iter().map(u64::to_string).collect::<Vec<_>>().join(","),
            ],
        );
    }
    t
}

/// Terminal view: the headline number, the other two ranges, the table.
pub fn render(r: &Report, now: i64, st: Style) -> String {
    let today = tokens::local_day(now);
    let sum = |f: fn(&Totals) -> u64| SOURCES.iter().map(|s| f(&totals(r, *s, today))).sum::<u64>();
    let (t, y, m) = (sum(|x| x.today), sum(|x| x.yesterday), sum(|x| x.month));
    let mut out = format!("  {} tokens today\n", st.bold(&format::count(t as f64)));
    out.push_str(&format!("  {} yesterday · {} in the last 30 days\n\n", format::count(y as f64), format::count(m as f64)));
    out.push_str(&table(r, now, st).tty(st));
    out.push_str(&format!("\n\n{}", st.muted("Counts include cached input tokens.")));
    out
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonDoc {
    schema: &'static str,
    generated_at: String,
    unit: &'static str,
    includes_cached_input: bool,
    totals: JsonTotals,
    tools: Vec<JsonTool>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonTotals {
    today: u64,
    yesterday: u64,
    last30_days: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonTool {
    id: &'static str,
    name: &'static str,
    log_files: usize,
    today: u64,
    yesterday: u64,
    last30_days: u64,
    /// Oldest first, ending today (local days).
    last14_days: Vec<u64>,
}

/// `quota.tokens.v2`. Field order is part of the contract.
pub fn json(r: &Report, now: i64) -> serde_json::Value {
    let today = tokens::local_day(now);
    let tools: Vec<JsonTool> = SOURCES
        .iter()
        .map(|s| {
            let x = totals(r, *s, today);
            JsonTool {
                id: s.id(),
                name: s.name(),
                log_files: r.get(*s).files,
                today: x.today,
                yesterday: x.yesterday,
                last30_days: x.month,
                last14_days: r.series(*s, today, 14),
            }
        })
        .collect();
    let doc = JsonDoc {
        schema: "quota.tokens.v2",
        generated_at: quota_core::time::format_rfc3339(now),
        unit: "tokens",
        includes_cached_input: true,
        totals: JsonTotals {
            today: tools.iter().map(|t| t.today).sum(),
            yesterday: tools.iter().map(|t| t.yesterday).sum(),
            last30_days: tools.iter().map(|t| t.last30_days).sum(),
        },
        tools,
    };
    serde_json::to_value(doc).unwrap_or(serde_json::Value::Null)
}

pub fn scan(ctx: &Ctx) -> (Report, i64) {
    // The first scan reads every log once; later runs read only what was
    // appended.
    let progress = "Reading session logs…";
    ctx.progress(progress);
    let now = quota_core::time::now();
    let report = tokens::scan(now);
    ctx.progress_done(progress);
    (report, now)
}

pub fn list(ctx: &Ctx) -> ExitCode {
    let (report, now) = scan(ctx);
    if ctx.json {
        return ctx.print_json(&json(&report, now));
    }
    if report.is_empty() {
        ctx.note("No session logs");
        ctx.note("  Quota reads Claude Code's (~/.claude/projects) and Codex's (~/.codex/sessions).");
        if !ctx.out_tty {
            return crate::out::print(&Table::new(table(&report, now, Style::PLAIN).headers, vec![]).tsv());
        }
        return ExitCode::SUCCESS;
    }
    if ctx.rich() {
        crate::out::print(&render(&report, now, ctx.out))
    } else {
        crate::out::print(&table(&report, now, Style::PLAIN).tsv())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(today: i64) -> Report {
        let mut r = Report::default();
        r.claude.files = 1;
        r.codex.files = 1;
        r.claude.days.insert(today, 2_400_000);
        r.claude.days.insert(today - 1, 1_000_000);
        r.codex.days.insert(today, 600_000);
        r
    }

    fn noon() -> i64 {
        quota_core::time::parse_rfc3339("2026-11-12T12:00:00Z").unwrap()
    }

    #[test]
    fn sparkline_scales_to_max() {
        assert_eq!(sparkline(&[0, 4, 8]), "▁▅█");
        assert_eq!(sparkline(&[0, 0]), "▁▁");
        assert_eq!(sparkline(&[1, 1000]), "▂█", "non-zero days never look empty");
    }

    #[test]
    fn terminal_view() {
        let now = noon();
        let t = render(&report(tokens::local_day(now)), now, Style::PLAIN);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "  3.0M tokens today");
        assert_eq!(lines[1], "  1.0M yesterday · 4.0M in the last 30 days");
        assert!(lines[3].starts_with("Tool") && lines[3].contains("Last 14 days"), "{t}");
        assert!(lines[4].starts_with("claude  2,400,000  1,000,000  3,400,000    85%"), "{t}");
        assert!(t.lines().all(|l| l == l.trim_end()));
    }

    #[test]
    fn piped_view_and_json() {
        let now = noon();
        let r = report(tokens::local_day(now));
        let tsv = table(&r, now, Style::PLAIN).tsv();
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines[0], "Tool\tToday\tYesterday\t30 days\tShare\tLast 14 days");
        assert!(lines[1].starts_with("claude\t2400000\t1000000\t3400000\t85\t0,0,"), "{tsv}");
        let v = json(&r, now);
        assert_eq!(v["schema"], "quota.tokens.v2");
        assert_eq!(v["totals"]["today"], 3_000_000);
        assert_eq!(v["tools"][0]["id"], "claude");
        assert_eq!(v["tools"][0]["last30Days"], 3_400_000);
        assert_eq!(v["tools"][1]["last14Days"].as_array().unwrap().len(), 14);
    }
}
