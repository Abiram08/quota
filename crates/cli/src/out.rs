//! One place for streams, errors, warnings, receipts and tables (Vercel CLI
//! guidelines, "Output" and "Failure and recovery").
//!
//! * stdout carries only the answer: a table, a value, or one JSON
//!   document. Progress, warnings, receipts and recovery go to stderr.
//! * With `--json`, stdout holds exactly one document and stderr stays
//!   empty, except for the single-line error document on failure.
//! * Exit codes: 0 done (empty results too), 1 operational failure,
//!   2 usage error, 130 cancelled.

use crate::style::{self, gutter, Style};
use serde_json::{json, Value};
use std::io::{IsTerminal, Write};
use std::process::ExitCode;

/// Resolved once per run; commands never check terminals themselves.
#[derive(Debug, Clone, Copy)]
pub struct Ctx {
    pub json: bool,
    pub out_tty: bool,
    pub err_tty: bool,
    pub out: Style,
    pub err: Style,
}

impl Ctx {
    pub fn detect(json: bool, no_color: bool) -> Ctx {
        Ctx {
            json,
            out_tty: std::io::stdout().is_terminal(),
            err_tty: std::io::stderr().is_terminal(),
            out: if json { Style::PLAIN } else { style::detect(no_color, false) },
            err: style::detect(no_color, true),
        }
    }

    /// Rich output (aligned table, summary, bars) only on a terminal;
    /// piped stdout gets a tab-separated table.
    pub fn rich(&self) -> bool {
        self.out_tty && !self.json
    }

    /// A progress line on stderr. On a terminal it is cleared by
    /// [`Ctx::progress_done`]; otherwise it stays as one plain line.
    /// Nothing with `--json`.
    pub fn progress(&self, text: &str) {
        if self.json {
            return;
        }
        let mut e = std::io::stderr().lock();
        if self.err_tty {
            let _ = write!(e, "{}", self.err.muted(text));
        } else {
            let _ = writeln!(e, "{}", text.replace('…', "..."));
        }
        let _ = e.flush();
    }

    pub fn progress_done(&self, text: &str) {
        if !self.json && self.err_tty {
            let _ = write!(std::io::stderr().lock(), "\r{}\r", " ".repeat(style::width(text)));
        }
    }

    /// A warning on stderr (human output only; JSON carries `warnings`).
    pub fn warn(&self, w: &Warning) {
        if self.json {
            return;
        }
        let mut e = std::io::stderr().lock();
        let _ = writeln!(e, "{}", gutter(&self.err.warn(), &w.message));
        if let Some((line, cmd)) = &w.next {
            let _ = writeln!(e, "  {line}\n      {}", self.err.command(cmd));
        }
    }

    /// The receipt for completed work, on stderr (`✓ Refreshed 3 providers`).
    pub fn receipt(&self, text: &str) {
        if !self.json {
            let _ = writeln!(std::io::stderr().lock(), "{}", gutter(&self.err.done(), text));
        }
    }

    /// Human prose about the result that is not the result itself (an
    /// empty collection, a note) goes to stderr.
    pub fn note(&self, text: &str) {
        if !self.json {
            let _ = writeln!(std::io::stderr().lock(), "{text}");
        }
    }

    /// Prints a JSON document: indented on a terminal, one line otherwise.
    pub fn print_json(&self, v: &Value) -> ExitCode {
        let text = if self.out_tty { serde_json::to_string_pretty(v) } else { serde_json::to_string(v) };
        print(&text.unwrap_or_default())
    }
}

/// Writes the answer to stdout. A closed pipe (`quota | head`) is not an
/// error.
pub fn print(text: &str) -> ExitCode {
    let mut out = std::io::stdout().lock();
    match writeln!(out, "{text}").and_then(|_| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub reason: &'static str,
    pub message: String,
    /// `(recovery line ending in ":", command)`.
    pub next: Option<(String, String)>,
}

impl Warning {
    pub fn json(&self) -> Value {
        json!({"reason": self.reason, "message": self.message})
    }
}

/// A failed run: what failed, the cause, and at most two recovery
/// commands. Never contains a token or a raw payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    /// `error` or `action_required` (blocked before any effect; a step by
    /// the user lets it run).
    pub status: &'static str,
    pub reason: &'static str,
    pub message: String,
    /// `(recovery line ending in ":", command)`.
    pub next: Vec<(String, String)>,
    pub exit: u8,
}

impl Failure {
    /// A usage error (exit 2).
    pub fn usage(reason: &'static str, message: String, next: Vec<(String, String)>) -> Failure {
        Failure { status: "error", reason, message, next, exit: 2 }
    }

    /// An operational failure (exit 1).
    pub fn failed(reason: &'static str, message: String, next: Vec<(String, String)>) -> Failure {
        Failure { status: "error", reason, message, next, exit: 1 }
    }

    /// Blocked before any effect until the user does something (exit 1).
    pub fn action(reason: &'static str, message: String, next: Vec<(String, String)>) -> Failure {
        Failure { status: "action_required", reason, message, next, exit: 1 }
    }

    pub fn json(&self) -> Value {
        let next: Vec<Value> = self
            .next
            .iter()
            .map(|(_, cmd)| json!({"command": cmd, "argv": cmd.split(' ').collect::<Vec<_>>()}))
            .collect();
        json!({"status": self.status, "reason": self.reason, "message": self.message, "next": next})
    }

    /// Human layout: headline, blank line, recovery line, command
    /// indented four spaces. With `--json`: one line on stderr.
    pub fn report(&self, ctx: &Ctx) -> ExitCode {
        let mut e = std::io::stderr().lock();
        if ctx.json {
            let _ = writeln!(e, "{}", serde_json::to_string(&self.json()).unwrap_or_default());
        } else {
            let _ = writeln!(e, "{}", gutter(&ctx.err.fail(), &self.message));
            for (line, cmd) in self.next.iter().take(2) {
                let _ = writeln!(e, "\n{line}\n    {}", ctx.err.command(cmd));
            }
        }
        ExitCode::from(self.exit)
    }
}

/// One table cell as shown on a terminal. Piped output uses the plain
/// values given to [`Table::push`] instead.
#[derive(Debug, Clone)]
pub struct Cell {
    pub styled: String,
}

impl Cell {
    /// Plain text from anywhere (cleaned of control characters).
    pub fn new(s: impl Into<String>) -> Cell {
        Cell { styled: style::clean(&s.into()) }
    }

    /// Already styled by Quota (bars, colours); the caller cleaned any
    /// outside text in it.
    pub fn styled(styled: String) -> Cell {
        Cell { styled }
    }
}

/// A borderless table. On a terminal: aligned columns, two spaces apart,
/// numbers right-aligned. Piped: the header row, then one row per line
/// with exactly one tab between columns, never padded or cut.
pub struct Table {
    pub headers: Vec<&'static str>,
    pub right: Vec<bool>,
    pub rows: Vec<Vec<Cell>>,
    /// The plain values used for TSV, when they differ from the cells
    /// (ISO times instead of "in 4h 0m").
    pub tsv: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: Vec<&'static str>, right: Vec<bool>) -> Table {
        Table { headers, right, rows: Vec::new(), tsv: Vec::new() }
    }

    pub fn push(&mut self, row: Vec<Cell>, tsv: Vec<String>) {
        self.rows.push(row);
        self.tsv.push(tsv.into_iter().map(|v| style::clean(&v)).collect());
    }

    pub fn tty(&self, st: Style) -> String {
        let n = self.headers.len();
        let mut w: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for r in &self.rows {
            for (i, c) in r.iter().enumerate().take(n) {
                w[i] = w[i].max(style::width(&c.styled));
            }
        }
        let line = |cells: Vec<String>| -> String {
            let mut parts: Vec<String> = Vec::new();
            for (i, c) in cells.iter().enumerate() {
                let last = i + 1 == cells.len();
                parts.push(if self.right[i] {
                    style::rpad(c, w[i])
                } else if last {
                    c.clone()
                } else {
                    style::pad(c, w[i])
                });
            }
            parts.join("  ").trim_end().to_string()
        };
        let mut out = vec![line(self.headers.iter().map(|h| st.bold(h)).collect())];
        for r in &self.rows {
            out.push(line(r.iter().map(|c| c.styled.clone()).collect()));
        }
        out.join("\n")
    }

    pub fn tsv(&self) -> String {
        let mut out = vec![self.headers.join("\t")];
        for r in &self.tsv {
            out.push(r.join("\t"));
        }
        out.join("\n")
    }
}

/// `1,234,567` (exact counts in tables; commas from 1,000).
pub fn commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `1 provider`, `3 providers` (never `provider(s)`).
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(999), "999");
        assert_eq!(commas(1000), "1,000");
        assert_eq!(commas(2_400_000), "2,400,000");
        assert_eq!(plural(1, "provider", "providers"), "1 provider");
        assert_eq!(plural(0, "provider", "providers"), "0 providers");
    }

    #[test]
    fn tables_align_on_a_terminal_and_tab_when_piped() {
        let mut t = Table::new(vec!["Tool", "Today"], vec![false, true]);
        t.push(vec![Cell::new("claude"), Cell::new("2,400,000")], vec!["claude".into(), "2400000".into()]);
        t.push(vec![Cell::new("codex"), Cell::new("0")], vec!["codex".into(), "0".into()]);
        assert_eq!(t.tty(Style::PLAIN), "Tool        Today\nclaude  2,400,000\ncodex           0");
        assert_eq!(t.tsv(), "Tool\tToday\nclaude\t2400000\ncodex\t0");
        let mut bad = Table::new(vec!["A"], vec![false]);
        bad.push(vec![Cell::new("x\ty")], vec!["x\ty\u{1b}".into()]);
        assert_eq!(bad.tsv(), "A\nx y", "a value never adds a column");
    }

    #[test]
    fn failure_json_is_one_line_with_next() {
        let f = Failure::action(
            "no_snapshot",
            "Can't list limits. There is no snapshot yet.".into(),
            vec![("To fetch usage, run:".into(), "quota refresh".into())],
        );
        let v = f.json();
        assert_eq!(v["status"], "action_required");
        assert_eq!(v["next"][0]["argv"], json!(["quota", "refresh"]));
        assert!(!serde_json::to_string(&v).unwrap().contains('\n'));
        assert_eq!(f.exit, 1);
    }
}
