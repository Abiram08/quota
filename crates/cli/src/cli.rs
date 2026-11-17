//! Command metadata, argument parsing and help, all from one table so help,
//! `--help --json` and the parser never disagree (Vercel CLI guidelines,
//! "Designing commands").
//!
//! Grammar: `quota <group> <action> [options]`. Groups are plural nouns
//! (`limits`, `tokens`, `tools`, `config`); top-level verbs are kept for
//! work on this PC (`refresh`, `status`) and tools named for what they do
//! (`mcp`). `ls` is accepted for every `list` and singular group names for
//! their plural, but help only teaches the canonical names.

use crate::out::Failure;
use crate::style::{pad, Style};
use quota_core::providers::ALL_IDS;

/// Global options: valid on every command.
pub const GLOBAL_OPTIONS: &[Opt] = &[
    Opt { long: "help", short: Some('h'), value: None, summary: "Show help for a command" },
    Opt { long: "json", short: None, value: None, summary: "Print one JSON document on stdout" },
    Opt { long: "no-color", short: None, value: None, summary: "Turn off color (NO_COLOR and FORCE_COLOR also work)" },
    Opt { long: "version", short: Some('v'), value: None, summary: "Print the version" },
];

const PROVIDER: Opt = Opt {
    long: "provider",
    short: None,
    value: Some("<PROVIDER>"),
    summary: "Comma-separated: claude, codex, cursor, copilot, openrouter",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opt {
    pub long: &'static str,
    pub short: Option<char>,
    pub value: Option<&'static str>,
    pub summary: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Id {
    LimitsList,
    Refresh,
    Status,
    TokensList,
    ToolsList,
    ConfigPath,
    ConfigInit,
    Mcp,
}

#[derive(Debug)]
pub struct Cmd {
    pub id: Id,
    pub path: &'static [&'static str],
    /// Root help group.
    pub group: &'static str,
    pub summary: &'static str,
    /// Second line of leaf help: a consequence or prerequisite.
    pub detail: &'static str,
    pub options: &'static [Opt],
    /// `(description, command)` pairs.
    pub examples: &'static [(&'static str, &'static str)],
    pub notes: &'static [&'static str],
    /// The `schema` of this command's `--json` document.
    pub schema: Option<&'static str>,
}

pub const COMMANDS: &[Cmd] = &[
    Cmd {
        id: Id::LimitsList,
        path: &["limits", "list"],
        group: "Limits",
        summary: "List each limit, what is left, and when it resets",
        detail: "Reads the snapshot saved on this PC. Makes no network requests.",
        options: &[PROVIDER],
        examples: &[
            ("List every limit", "quota limits list"),
            ("Only Claude Code and Codex, as JSON", "quota limits list --provider claude,codex --json"),
        ],
        notes: &[
            "On a terminal this prints a summary and a table. Piped, it prints a tab-separated table with a header row and ISO 8601 UTC times.",
            "quota-tray keeps the snapshot up to date. Without it, run quota refresh.",
        ],
        schema: Some("quota.limits.v2"),
    },
    Cmd {
        id: Id::Refresh,
        path: &["refresh"],
        group: "Limits",
        summary: "Fetch usage from each provider now and save it",
        detail: "Calls each provider's own usage endpoint with the sign-in already saved on this PC.",
        options: &[PROVIDER],
        examples: &[("Fetch fresh usage for Codex", "quota refresh --provider codex")],
        notes: &[
            "Providers that aren't connected (not signed in, no plan, an encrypted sign-in) are skipped. Exits 1 only if a connected provider failed.",
            "Quota reads other tools' sign-ins read-only and never changes them.",
        ],
        schema: Some("quota.limits.v2"),
    },
    Cmd {
        id: Id::Status,
        path: &["status"],
        group: "Limits",
        summary: "Print one line about the most urgent limit",
        detail: "For shell prompts and status lines. Reads the saved snapshot only.",
        options: &[PROVIDER],
        examples: &[("Show the most urgent limit", "quota status")],
        notes: &[],
        schema: Some("quota.status.v1"),
    },
    Cmd {
        id: Id::TokensList,
        path: &["tokens", "list"],
        group: "Tokens",
        summary: "List tokens used today, yesterday and in 30 days",
        detail: "Counted from Claude Code's and Codex's session logs on this PC.",
        options: &[],
        examples: &[("List tokens by tool", "quota tokens list")],
        notes: &[
            "Counts include cached input tokens.",
            "The first run reads every log once; later runs read only what was added.",
        ],
        schema: Some("quota.tokens.v2"),
    },
    Cmd {
        id: Id::ToolsList,
        path: &["tools", "list"],
        group: "This PC",
        summary: "List AI tools on this PC and which Quota meters",
        detail: "Only checks whether each tool's own files exist.",
        options: &[],
        examples: &[("List AI coding tools", "quota tools list")],
        notes: &["A tool is metered only when its usage source was verified on Windows. Quota does not guess one."],
        schema: Some("quota.tools.v1"),
    },
    Cmd {
        id: Id::ConfigPath,
        path: &["config", "path"],
        group: "This PC",
        summary: "Print the config file's path",
        detail: "Does not create the file.",
        options: &[],
        examples: &[("Print the config file's path", "quota config path")],
        notes: &[],
        schema: Some("quota.config.v1"),
    },
    Cmd {
        id: Id::ConfigInit,
        path: &["config", "init"],
        group: "This PC",
        summary: "Create the config file if it is missing",
        detail: "An existing file is never changed.",
        options: &[],
        examples: &[("Create the config file", "quota config init")],
        notes: &[],
        schema: Some("quota.config.v1"),
    },
    Cmd {
        id: Id::Mcp,
        path: &["mcp"],
        group: "Agents",
        summary: "Run Quota as an MCP server on stdin and stdout",
        detail: "Started by an MCP client such as Claude Code, not run by hand.",
        options: &[],
        examples: &[("Add Quota to Claude Code", "claude mcp add quota -- quota mcp")],
        notes: &[
            "Tools: limits_list, limits_refresh, tokens_list, tools_list. They return the same documents as --json.",
            "Only limits_refresh uses the network, and only to each provider's own usage endpoint.",
        ],
        schema: None,
    },
];

const GROUPS: [&str; 4] = ["Limits", "Tokens", "This PC", "Agents"];

/// Groups and their default action (`quota limits` = `quota limits list`).
const DEFAULTS: [(&str, Option<Id>); 4] =
    [("limits", Some(Id::LimitsList)), ("tokens", Some(Id::TokensList)), ("tools", Some(Id::ToolsList)), ("config", None)];

pub fn command(id: Id) -> &'static Cmd {
    COMMANDS.iter().find(|c| c.id == id).expect("every Id has metadata")
}

/// What one invocation asks for.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    /// None with `help` set: root help. None otherwise: the default command.
    pub cmd: Option<Id>,
    /// A group named without an action, shown as help (`quota config`).
    pub group_help: Option<&'static str>,
    pub json: bool,
    pub no_color: bool,
    pub help: bool,
    pub version: bool,
    pub providers: Option<Vec<String>>,
}

impl Args {
    pub fn id(&self) -> Id {
        self.cmd.unwrap_or(Id::LimitsList)
    }
}

fn canonical_word(w: &str) -> &str {
    match w {
        "limit" => "limits",
        "token" => "tokens",
        "tool" => "tools",
        "ls" => "list",
        other => other,
    }
}

fn path_text(path: &[&str]) -> String {
    if path.is_empty() {
        "quota".into()
    } else {
        format!("quota {}", path.join(" "))
    }
}

/// Closest command within two edits (`tokenz` -> `tokens`).
fn did_you_mean(word: &str) -> Option<&'static str> {
    fn distance(a: &str, b: &str) -> usize {
        let b: Vec<char> = b.chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.chars().enumerate() {
            let mut cur = vec![i + 1];
            for (j, cb) in b.iter().enumerate() {
                let cost = usize::from(ca != *cb);
                cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
            }
            prev = cur;
        }
        prev[b.len()]
    }
    const WORDS: [&str; 8] = ["limits", "refresh", "status", "tokens", "tools", "config", "mcp", "help"];
    WORDS.iter().map(|c| (distance(word, c), *c)).filter(|(d, _)| *d <= 2).min_by_key(|(d, _)| *d).map(|(_, c)| c)
}

fn parse_providers(v: &str) -> Result<Vec<String>, Failure> {
    let mut out: Vec<String> = Vec::new();
    for raw in v.split(',') {
        let id = raw.trim().to_ascii_lowercase();
        if id.is_empty() {
            continue;
        }
        if !ALL_IDS.contains(&id.as_str()) {
            return Err(Failure::usage(
                "invalid_arguments",
                format!("Can't use --provider {}. Use one of: {}.", crate::style::clean(raw.trim()), ALL_IDS.join(", ")),
                vec![],
            ));
        }
        if !out.contains(&id) {
            out.push(id);
        }
    }
    if out.is_empty() {
        return Err(Failure::usage(
            "missing_value",
            "Can't use --provider without a value.".into(),
            vec![("For example:".into(), "quota limits list --provider claude".into())],
        ));
    }
    Ok(out)
}

/// Parses argv (without the program name). Help wins over every other
/// check once it is recognised.
pub fn parse(argv: &[String]) -> Result<Args, Failure> {
    let mut a = Args::default();
    let mut words: Vec<String> = Vec::new();
    let mut options: Vec<(String, Option<String>)> = Vec::new();
    let mut it = argv.iter().peekable();
    let mut literal = false;
    while let Some(arg) = it.next() {
        if literal || !arg.starts_with('-') || arg == "-" {
            words.push(arg.clone());
            continue;
        }
        if arg == "--" {
            literal = true;
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (arg.clone(), None),
        };
        match name.as_str() {
            "-h" | "--help" => a.help = true,
            "-v" | "--version" => a.version = true,
            "--json" => a.json = !matches!(inline.as_deref(), Some("false")),
            "--no-color" => a.no_color = true,
            "--provider" => {
                let v = match inline {
                    Some(v) => Some(v),
                    None => it.next_if(|n| !n.starts_with('-')).cloned(),
                };
                options.push(("--provider".into(), v));
            }
            _ => options.push((name, inline)),
        }
    }

    // Resolve the command path.
    let mut rest = words.iter().map(|w| canonical_word(w)).collect::<Vec<_>>().into_iter();
    let first = rest.next();
    let mut help_for: Option<Vec<&str>> = None;
    let mut consumed: Vec<&str> = Vec::new();
    match first {
        None => {}
        Some("help") => {
            a.help = true;
            help_for = Some(rest.by_ref().collect());
        }
        Some(w) => {
            consumed.push(w);
            if let Some((g, default)) = DEFAULTS.iter().find(|(g, _)| *g == w) {
                match rest.next() {
                    None => match default {
                        Some(id) => a.cmd = Some(*id),
                        None => a.group_help = Some(g),
                    },
                    Some(action) => {
                        consumed.push(action);
                        match COMMANDS.iter().find(|c| c.path == [*g, action]) {
                            Some(c) => a.cmd = Some(c.id),
                            None if a.help => a.group_help = Some(g),
                            None => {
                                return Err(Failure::usage(
                                    "unknown_command",
                                    format!("Unknown command {} for quota {g}.", crate::style::clean(action)),
                                    vec![("To see its commands, run:".into(), format!("quota {g} --help"))],
                                ))
                            }
                        }
                    }
                }
            } else if let Some(c) = COMMANDS.iter().find(|c| c.path == [w]) {
                a.cmd = Some(c.id);
            } else if !a.help {
                let w = crate::style::clean(w);
                let next = if ALL_IDS.contains(&w.to_ascii_lowercase().as_str()) {
                    vec![("To list one provider's limits, run:".into(), format!("quota limits list --provider {}", w.to_ascii_lowercase()))]
                } else if let Some(m) = did_you_mean(&w) {
                    vec![(format!("Did you mean {m}? To see it, run:"), format!("quota {m} --help"))]
                } else {
                    vec![("To see the commands, run:".into(), "quota --help".into())]
                };
                return Err(Failure::usage("unknown_command", format!("Unknown command {w} for quota."), next));
            }
        }
    }
    if a.help {
        // Help for `help <path>` or for the path given with --help.
        if let Some(path) = help_for {
            resolve_help_path(&mut a, &path)?;
        }
        return Ok(a);
    }
    if a.version {
        return Ok(a);
    }
    if let Some(extra) = rest.next() {
        let at = path_text(&consumed);
        return Err(Failure::usage(
            "invalid_arguments",
            format!("Unexpected argument {} for {at}.", crate::style::clean(extra)),
            vec![("To see what it takes, run:".into(), format!("{at} --help"))],
        ));
    }
    if a.group_help.is_some() {
        return Ok(a);
    }

    // Options: only what the command declares.
    let cmd = command(a.id());
    let at = path_text(if a.cmd.is_none() { &[] } else { cmd.path });
    for (name, value) in options {
        let known = cmd.options.iter().find(|o| o.long == name.trim_start_matches('-'));
        match (known, name.as_str()) {
            (Some(o), _) if o.long == "provider" => {
                if a.providers.is_some() {
                    return Err(Failure::usage(
                        "invalid_arguments",
                        "Can't use --provider twice. Give one comma-separated list.".into(),
                        vec![("For example:".into(), format!("{} --provider claude,codex", path_text(cmd.path)))],
                    ));
                }
                a.providers = Some(parse_providers(value.as_deref().unwrap_or(""))?);
            }
            (_, "--refresh" | "-r") => {
                return Err(Failure::usage(
                    "unknown_option",
                    format!("Unknown option {name} for {at}."),
                    vec![("To fetch fresh usage, run:".into(), "quota refresh".into())],
                ))
            }
            _ => {
                return Err(Failure::usage(
                    "unknown_option",
                    format!("Unknown option {} for {at}.", crate::style::clean(&name)),
                    vec![("To see its options, run:".into(), format!("{at} --help"))],
                ))
            }
        }
    }
    Ok(a)
}

fn resolve_help_path(a: &mut Args, path: &[&str]) -> Result<(), Failure> {
    match path {
        [] => {}
        [g] if DEFAULTS.iter().any(|(d, _)| d == g) => {
            a.group_help = DEFAULTS.iter().find(|(d, _)| d == g).map(|(d, _)| *d);
        }
        _ => match COMMANDS.iter().find(|c| c.path == path) {
            Some(c) => a.cmd = Some(c.id),
            None => {
                return Err(Failure::usage(
                    "unknown_command",
                    format!("Unknown command {} for quota.", crate::style::clean(&path.join(" "))),
                    vec![("To see the commands, run:".into(), "quota --help".into())],
                ))
            }
        },
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Help

fn opt_syntax(o: &Opt) -> String {
    let short = o.short.map(|c| format!("-{c}, ")).unwrap_or_else(|| "    ".into());
    match o.value {
        Some(v) => format!("{short}--{} {v}", o.long),
        None => format!("{short}--{}", o.long),
    }
}

/// Borderless two-column rows; a row whose syntax is too wide for the
/// column puts its description on the next line instead of squeezing it.
fn rows(st: Style, items: &[(String, String)]) -> String {
    const COL: usize = 18;
    let mut out = String::new();
    for (syntax, desc) in items {
        if syntax.chars().count() + 2 > COL {
            out.push_str(&format!("  {}\n  {}{desc}\n", st.bold(syntax), " ".repeat(COL)));
        } else {
            out.push_str(&format!("  {}{desc}\n", pad(&st.bold(syntax), COL)));
        }
    }
    out
}

fn examples(st: Style, items: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (i, (what, cmd)) in items.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("  {what}\n    {}\n", st.command(cmd)));
    }
    out
}

pub fn root_help(st: Style) -> String {
    let mut out = format!(
        "{} {}\n\n{}\n",
        st.bold("quota"),
        "[command] [options]",
        st.muted("See what is left of your AI coding limits and when each resets.")
    );
    for g in GROUPS {
        let items: Vec<(String, String)> = COMMANDS
            .iter()
            .filter(|c| c.group == g)
            .map(|c| {
                let default = if c.id == Id::LimitsList { " (default)" } else { "" };
                (c.path.join(" "), format!("{}{default}", c.summary))
            })
            .collect();
        out.push_str(&format!("\n{}\n{}", st.heading(&format!("{g}:")), rows(st, &items)));
    }
    let globals: Vec<(String, String)> = GLOBAL_OPTIONS.iter().map(|o| (opt_syntax(o), o.summary.into())).collect();
    out.push_str(&format!("\n{}\n{}", st.heading("Global Options:"), rows(st, &globals)));
    out.push_str(&format!(
        "\n{}\n{}",
        st.heading("Examples:"),
        examples(
            st,
            &[
                ("Check your limits", "quota"),
                ("Fetch fresh usage for Claude Code and Codex", "quota refresh --provider claude,codex"),
                ("Add Quota to Claude Code as an MCP server", "claude mcp add quota -- quota mcp"),
            ]
        )
    ));
    out.trim_end().to_string()
}

pub fn group_help(st: Style, group: &str) -> String {
    let items: Vec<(String, String)> = COMMANDS
        .iter()
        .filter(|c| c.path.len() == 2 && c.path[0] == group)
        .map(|c| {
            let default = DEFAULTS.iter().any(|(g, d)| *g == group && *d == Some(c.id));
            (c.path[1].to_string(), format!("{}{}", c.summary, if default { " (default)" } else { "" }))
        })
        .collect();
    let mut out = format!("{} {}\n", st.bold(&format!("quota {group}")), "<command> [options]");
    out.push_str(&format!("\n{}\n{}", st.heading("Commands:"), rows(st, &items)));
    out.push_str(&format!("\n{}\n  Run quota --help for the global options.", st.heading("Global Options:")));
    out
}

pub fn command_help(st: Style, c: &Cmd) -> String {
    let mut out = format!("{} [options]\n\n{}\n{}\n", st.bold(&path_text(c.path)), st.muted(c.summary), wrap(c.detail, 0));
    if !c.options.is_empty() {
        let mut opts: Vec<&Opt> = c.options.iter().collect();
        opts.sort_by_key(|o| o.long);
        let items: Vec<(String, String)> = opts.iter().map(|o| (opt_syntax(o), o.summary.into())).collect();
        out.push_str(&format!("\n{}\n{}", st.heading("Options:"), rows(st, &items)));
    }
    out.push_str(&format!("\n{}\n  Run quota --help for the global options.\n", st.heading("Global Options:")));
    if !c.examples.is_empty() {
        out.push_str(&format!("\n{}\n{}", st.heading("Examples:"), examples(st, c.examples)));
    }
    if !c.notes.is_empty() {
        out.push_str(&format!("\n{}\n", st.heading("Notes:")));
        for n in c.notes {
            out.push_str(&wrap(n, 2));
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

/// Word-wraps prose to 80 columns at `indent`. Commands and option names
/// never sit in this prose, so breaking at spaces cannot split one.
pub(crate) fn wrap(text: &str, indent: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if !line.is_empty() && indent + line.chars().count() + 1 + word.chars().count() > 80 {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    lines.push(line);
    lines.iter().map(|l| format!("{}{l}", " ".repeat(indent))).collect::<Vec<_>>().join("\n")
}

fn opt_json(o: &Opt) -> serde_json::Value {
    let accepted: Option<Vec<&str>> = (o.long == "provider").then(|| ALL_IDS.to_vec());
    serde_json::json!({
        "name": format!("--{}", o.long),
        "short": o.short.map(|c| format!("-{c}")),
        "type": if o.value.is_some() { "string" } else { "boolean" },
        "valueHint": o.value,
        "default": null,
        "acceptedValues": accepted,
        "repeatable": false,
        "list": o.long == "provider",
    })
}

fn cmd_json(c: &Cmd) -> serde_json::Value {
    serde_json::json!({
        "path": c.path,
        "summary": c.summary,
        "options": c.options.iter().map(opt_json).collect::<Vec<_>>(),
        "resultSchema": c.schema,
    })
}

/// `--help --json`: the same metadata as human help.
pub fn help_json(a: &Args) -> serde_json::Value {
    match (a.cmd, a.group_help) {
        (Some(id), _) => cmd_json(command(id)),
        (None, Some(g)) => serde_json::json!({
            "path": [g],
            "summary": null,
            "subcommands": COMMANDS.iter().filter(|c| c.path[0] == g).map(cmd_json).collect::<Vec<_>>(),
        }),
        (None, None) => serde_json::json!({
            "path": [],
            "summary": "See what is left of your AI coding limits and when each resets.",
            "globalOptions": GLOBAL_OPTIONS.iter().map(opt_json).collect::<Vec<_>>(),
            "subcommands": COMMANDS.iter().map(cmd_json).collect::<Vec<_>>(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn ok(s: &str) -> Args {
        parse(&argv(s)).unwrap_or_else(|e| panic!("{s}: {}", e.message))
    }

    fn err(s: &str) -> Failure {
        parse(&argv(s)).expect_err(s)
    }

    #[test]
    fn commands_and_defaults() {
        assert_eq!(ok("").id(), Id::LimitsList);
        assert_eq!(ok("limits").id(), Id::LimitsList);
        assert_eq!(ok("limits ls").id(), Id::LimitsList);
        assert_eq!(ok("limit list").id(), Id::LimitsList);
        assert_eq!(ok("refresh").id(), Id::Refresh);
        assert_eq!(ok("status").id(), Id::Status);
        assert_eq!(ok("tokens").id(), Id::TokensList);
        assert_eq!(ok("tools list").id(), Id::ToolsList);
        assert_eq!(ok("config path").id(), Id::ConfigPath);
        assert_eq!(ok("config init").id(), Id::ConfigInit);
        assert_eq!(ok("mcp").id(), Id::Mcp);
        assert_eq!(ok("config").group_help, Some("config"));
    }

    #[test]
    fn options_and_globals() {
        let a = ok("limits list --provider Claude,codex,claude --json");
        assert_eq!(a.providers, Some(vec!["claude".to_string(), "codex".to_string()]));
        assert!(a.json);
        assert_eq!(ok("--provider=codex refresh").providers, Some(vec!["codex".into()]));
        assert!(ok("--no-color tokens").no_color);
        assert!(ok("tools -v").version);
        assert!(!ok("--json=false").json);
    }

    #[test]
    fn help_wins_and_has_every_entry_point() {
        assert!(ok("--help").help && ok("-h").help && ok("help").help);
        assert_eq!(ok("help limits list").cmd, Some(Id::LimitsList));
        assert_eq!(ok("refresh --help --bogus").cmd, Some(Id::Refresh), "help skips other validation");
        assert_eq!(ok("help config").group_help, Some("config"));
        assert!(ok("nope --help").help);
    }

    #[test]
    fn usage_errors_name_the_token_and_exit_2() {
        let e = err("claude");
        assert_eq!((e.reason, e.exit), ("unknown_command", 2));
        assert_eq!(e.message, "Unknown command claude for quota.");
        assert_eq!(e.next[0].1, "quota limits list --provider claude");
        let e = err("tokenz");
        assert_eq!(e.next[0].0, "Did you mean tokens? To see it, run:");
        let e = err("tokens --provider claude");
        assert_eq!((e.reason, e.message.as_str()), ("unknown_option", "Unknown option --provider for quota tokens list."));
        assert_eq!(err("--refresh").next[0].1, "quota refresh");
        assert_eq!(err("limits list extra").reason, "invalid_arguments");
        assert_eq!(err("limits list --provider nope").reason, "invalid_arguments");
        assert_eq!(err("limits list --provider").reason, "missing_value");
        assert_eq!(err("refresh --provider claude --provider codex").reason, "invalid_arguments");
        assert_eq!(err("config wipe").reason, "unknown_command");
    }

    #[test]
    fn examples_parse() {
        for c in COMMANDS {
            for (_, ex) in c.examples {
                if let Some(rest) = ex.strip_prefix("quota ") {
                    ok(rest);
                }
            }
        }
        ok("refresh --provider claude,codex");
    }

    #[test]
    fn help_text_is_plain_without_colour() {
        let t = root_help(Style::PLAIN);
        assert!(t.starts_with("quota [command] [options]\n\n"), "{t}");
        assert!(t.contains("\nLimits:\n  limits list"), "{t}");
        assert!(t.contains("(default)"));
        assert!(t.contains("\nGlobal Options:\n  -h, --help"), "{t}");
        assert!(t.contains("\nExamples:\n  Check your limits\n    quota\n"), "{t}");
        let fits = |t: &str| t.lines().all(|l| l.chars().count() <= 80);
        assert!(fits(&t), "{t}");
        for c in COMMANDS {
            let h = command_help(Style::PLAIN, c);
            assert!(fits(&h), "{h}");
        }
        let c = command_help(Style::PLAIN, command(Id::Refresh));
        assert!(c.starts_with("quota refresh [options]\n"), "{c}");
        assert!(c.contains("Options:\n      --provider <PROVIDER>\n"), "{c}");
        let v = help_json(&ok("limits list --help"));
        assert_eq!(v["resultSchema"], "quota.limits.v2");
        assert_eq!(v["options"][0]["name"], "--provider");
    }
}
