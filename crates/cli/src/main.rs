//! `quota`: what is left of your AI coding limits, from the snapshot on
//! this PC. Only `quota refresh` (and the MCP tool `limits_refresh`) use
//! the network, and only to each provider's own usage endpoint.
//!
//! Follows the Vercel CLI design guidelines (<https://vercel.com/design/cli>):
//! one table of commands drives parsing and help (`cli.rs`); one module
//! owns streams, errors and tables (`out.rs`).

mod cli;
mod limits_cmd;
mod mcp;
mod out;
mod style;
mod tokens_cmd;

use cli::Id;
use out::{Cell, Ctx, Failure, Table};
use quota_core::config::Config;
use quota_core::providers;
use serde_json::json;
use std::process::ExitCode;
use style::Style;

/// Ctrl+C ends the run with exit code 130 (the Windows default would be
/// 0xC000013A). Nothing Quota does needs cleanup: the snapshot is written
/// to a temp file and renamed into place.
#[cfg(windows)]
fn exit_130_on_ctrl_c() {
    use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT};
    unsafe extern "system" fn handler(kind: u32) -> windows_sys::core::BOOL {
        if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
            std::process::exit(130);
        }
        0
    }
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(not(windows))]
fn exit_130_on_ctrl_c() {}

fn main() -> ExitCode {
    exit_130_on_ctrl_c();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let json = argv.iter().any(|a| a == "--json");
    let no_color = argv.iter().any(|a| a == "--no-color");
    let args = match cli::parse(&argv) {
        Ok(a) => a,
        Err(f) => return f.report(&Ctx::detect(json, no_color)),
    };
    let ctx = Ctx::detect(args.json, args.no_color);

    if args.help {
        if args.json {
            return ctx.print_json(&cli::help_json(&args));
        }
        let st = ctx.out;
        let text = match (args.cmd, args.group_help) {
            (Some(id), _) => cli::command_help(st, cli::command(id)),
            (None, Some(g)) => cli::group_help(st, g),
            (None, None) => cli::root_help(st),
        };
        return out::print(&text);
    }
    if args.version {
        let v = env!("CARGO_PKG_VERSION");
        return if args.json { ctx.print_json(&json!({"version": v})) } else { out::print(v) };
    }
    if let Some(g) = args.group_help {
        // A group without a default action shows its help and exits 0.
        return out::print(&cli::group_help(ctx.out, g));
    }

    let only = args.providers.as_deref();
    match args.id() {
        Id::LimitsList => limits_cmd::list(&ctx, only),
        Id::Refresh => limits_cmd::run_refresh(&ctx, only),
        Id::Status => limits_cmd::status(&ctx, only),
        Id::TokensList => tokens_cmd::list(&ctx),
        Id::ToolsList => tools_list(&ctx),
        Id::ConfigPath => config(&ctx, false),
        Id::ConfigInit => config(&ctx, true),
        Id::Mcp => mcp::run(ctx.err_tty),
    }
}

// ---------------------------------------------------------------------------
// quota tools list

pub fn tools_json(found: &[providers::detect::Detected]) -> serde_json::Value {
    let tools: Vec<serde_json::Value> = found
        .iter()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "installed": d.installed,
                "metered": d.provider.is_some(),
                "status": d.status(),
            })
        })
        .collect();
    json!({"schema": "quota.tools.v1", "tools": tools})
}

fn tools_table(found: &[providers::detect::Detected], st: Style) -> Table {
    let mut t = Table::new(vec!["Tool", "Name", "Installed", "Metered"], vec![false, false, false, false]);
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    // Metered first, then the installed ones, then the rest.
    let mut sorted: Vec<&providers::detect::Detected> = found.iter().collect();
    sorted.sort_by_key(|d| (d.provider.is_none(), !d.installed));
    for d in sorted {
        let installed =
            if d.installed { Cell::styled(st.tone(style::Tone::Ok, "yes")) } else { Cell::new("no") };
        t.push(
            vec![Cell::new(d.id), Cell::new(d.name), installed, Cell::new(yes_no(d.provider.is_some()))],
            vec![d.id.into(), d.name.into(), yes_no(d.installed).into(), yes_no(d.provider.is_some()).into()],
        );
    }
    t
}

fn tools_list(ctx: &Ctx) -> ExitCode {
    let found = providers::detect::scan();
    if ctx.json {
        return ctx.print_json(&tools_json(&found));
    }
    let t = tools_table(&found, ctx.out);
    if !ctx.rich() {
        return out::print(&t.tsv());
    }
    let installed = found.iter().filter(|d| d.installed).count();
    let metered = found.iter().filter(|d| d.provider.is_some()).count();
    out::print(&format!(
        "  {installed} of {} AI coding tools are on this PC. Quota meters {metered}.\n\n{}\n\n{}",
        found.len(),
        t.tty(ctx.out),
        ctx.out.muted("Quota meters a tool only when its usage source was verified on Windows.")
    ))
}

// ---------------------------------------------------------------------------
// quota config path | init

fn config(ctx: &Ctx, init: bool) -> ExitCode {
    let Some(path) = quota_core::paths::config_file() else {
        return Failure::failed("missing_environment", "Can't find the config file. APPDATA is not set.".into(), vec![])
            .report(ctx);
    };
    let existed = path.exists();
    let mut outcome = None;
    if init && !existed {
        if let Err(e) = Config::write_default_if_missing() {
            return Failure::failed(
                "write_failed",
                format!("Couldn't create the config file. {}.", quota_core::output::sentence_case(&e.to_string())),
                vec![],
            )
            .report(ctx);
        }
        outcome = Some("created");
    } else if init {
        outcome = Some("unchanged");
    }
    let shown = path.display().to_string();
    if ctx.json {
        let mut v = json!({"schema": "quota.config.v1", "config": {"path": shown, "exists": path.exists()}});
        if let Some(o) = outcome {
            v["outcome"] = json!(o);
        }
        return ctx.print_json(&v);
    }
    match outcome {
        Some("created") => ctx.receipt("Created the config file with the defaults."),
        Some(_) => ctx.note("  The config file already exists. Nothing was changed."),
        None if !existed => ctx.warn(&out::Warning {
            reason: "config_missing",
            message: "The config file doesn't exist yet; Quota is using the defaults.".into(),
            next: Some(("To create it, run:".into(), "quota config init".into())),
        }),
        None => {}
    }
    out::print(&shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_table_lists_metered_first() {
        let found = providers::detect::scan();
        let t = tools_table(&found, Style::PLAIN).tsv();
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "Tool\tName\tInstalled\tMetered");
        assert_eq!(lines.len(), found.len() + 1);
        assert!(lines[1].ends_with("\tyes"), "{t}");
        let v = tools_json(&found);
        assert_eq!(v["schema"], "quota.tools.v1");
        assert_eq!(v["tools"].as_array().unwrap().len(), found.len());
    }
}
