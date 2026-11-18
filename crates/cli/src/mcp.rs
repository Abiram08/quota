//! `quota mcp`: a Model Context Protocol server on stdin/stdout, so an
//! agent (Claude Code, Codex, Cursor, ...) can ask what is left of the
//! user's limits. No SDK: newline-delimited JSON-RPC 2.0, as the stdio
//! transport specifies.
//!
//! Protocol versions (verified against modelcontextprotocol.io, 2026-11):
//! * `2026-07-28` (current, stateless): `server/discover`, and every
//!   request carries `_meta["io.modelcontextprotocol/protocolVersion"]`.
//! * `2025-11-25` and earlier: the `initialize` handshake, then requests.
//!   Most clients in use still speak this, so both are served. A request
//!   with modern `_meta` is answered statelessly; `initialize` selects the
//!   legacy behaviour for this process.
//!
//! Tools mirror the CLI (one capability, one name): `limits_list`,
//! `limits_refresh`, `tokens_list`, `tools_list`, and each returns the
//! same document as the matching `--json` command. Only `limits_refresh`
//! uses the network, and only each provider's own usage endpoint.
//!
//! stdout carries only protocol messages. Nothing is logged: no request,
//! argument or result is written anywhere.

use crate::limits_cmd;
use crate::out::Failure;
use quota_core::providers::ALL_IDS;
use quota_core::{output, time};
use serde_json::{json, Map, Value};
use std::io::{BufRead, Write};
use std::process::ExitCode;

const MODERN: [&str; 1] = ["2026-07-28"];
const LEGACY: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPS: &str = "io.modelcontextprotocol/clientCapabilities";
/// `limits_refresh` reuses a snapshot younger than this instead of
/// calling every provider again (the tray's own minimum interval).
const REFRESH_MIN_SECS: i64 = 60;

const INSTRUCTIONS: &str = "Quota reports the user's AI coding subscription limits (Claude Code, Codex, Cursor, \
GitHub Copilot, OpenRouter) and token use, from data on this PC. Call limits_list first: it is instant and makes \
no network requests. Its updatedAt says how old the data is; call limits_refresh only when that is old or the user \
asks for current numbers. percentLeft is what is left of a limit, resetsAt when it refills, and status says whether \
the current pace runs out first.";

fn server_info() -> Value {
    json!({"name": "quota", "title": "Quota", "version": env!("CARGO_PKG_VERSION")})
}

fn provider_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "provider": {
                "type": "array",
                "items": {"type": "string", "enum": ALL_IDS},
                "description": "Only these providers. Omit for all of them."
            }
        },
        "additionalProperties": false
    })
}

fn no_args() -> Value {
    json!({"type": "object", "additionalProperties": false})
}

pub fn tools() -> Value {
    json!([
        {
            "name": "limits_list",
            "title": "List limits",
            "description": "List the user's AI coding limits from the snapshot saved on this PC: how much of each is \
left (percentLeft), when it resets (resetsAt), and whether the current pace runs out before then (status, pace). \
No network requests. Check updatedAt for the data's age.",
            "inputSchema": provider_schema(),
            "annotations": {"title": "List limits", "readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "limits_refresh",
            "title": "Refresh limits",
            "description": "Fetch current usage from each provider's own usage endpoint, using the sign-in already \
saved on this PC, save it, and return the same document as limits_list plus a refresh summary. Uses the network. \
Reuses data fetched less than a minute ago. Never changes any sign-in.",
            "inputSchema": provider_schema(),
            "annotations": {
                "title": "Refresh limits",
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": true
            }
        },
        {
            "name": "tokens_list",
            "title": "List tokens",
            "description": "Tokens used today, yesterday and in the last 30 days, by tool, counted from Claude Code's \
and Codex's session logs on this PC. Includes cached input tokens. No network requests.",
            "inputSchema": no_args(),
            "annotations": {"title": "List tokens", "readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "tools_list",
            "title": "List AI tools",
            "description": "Which AI coding tools are installed on this PC, and which ones Quota can meter. Only checks \
whether each tool's files exist. No network requests.",
            "inputSchema": no_args(),
            "annotations": {"title": "List AI tools", "readOnlyHint": true, "openWorldHint": false}
        }
    ])
}

/// A tool's result: the document, or a message the model can act on.
enum Outcome {
    Doc(Value),
    Error(String),
}

/// The CLI's failure, retold for an agent (no shell commands to run).
fn tool_error(f: Failure) -> Outcome {
    let hint = match f.reason {
        "no_snapshot" | "invalid_snapshot" => " Call limits_refresh to fetch usage now.",
        "no_providers" => " Every provider is turned off in Quota's config file.",
        _ => "",
    };
    Outcome::Error(format!("{}{hint}", f.message))
}

fn providers_arg(args: &Map<String, Value>) -> Result<Option<Vec<String>>, String> {
    for k in args.keys() {
        if k != "provider" {
            return Err(format!("Unknown argument {k}. This tool takes only provider."));
        }
    }
    let list: Vec<&Value> = match args.get("provider") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(a)) => a.iter().collect(),
        Some(v @ Value::String(_)) => vec![v],
        Some(_) => return Err("provider must be a list of provider ids.".into()),
    };
    let mut ids: Vec<String> = Vec::new();
    for v in list {
        let id = v.as_str().map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
        if !ALL_IDS.contains(&id.as_str()) {
            return Err(format!("Unknown provider {}. Use one of: {}.", v, ALL_IDS.join(", ")));
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(if ids.is_empty() { None } else { Some(ids) })
}

fn call(name: &str, args: &Map<String, Value>) -> Option<Outcome> {
    let out = match name {
        "limits_list" => match providers_arg(args) {
            Err(m) => Outcome::Error(m),
            Ok(only) => match limits_cmd::load() {
                Err(f) => tool_error(f),
                Ok(snap) => {
                    let now = time::now();
                    let scoped = output::scoped(&snap, only.as_deref());
                    Outcome::Doc(limits_cmd::doc(&scoped, now, &limits_cmd::freshness(&scoped, now, false)))
                }
            },
        },
        "limits_refresh" => match providers_arg(args) {
            Err(m) => Outcome::Error(m),
            Ok(only) => refresh(only.as_deref()),
        },
        "tokens_list" | "tools_list" if !args.is_empty() => Outcome::Error("This tool takes no arguments.".into()),
        "tokens_list" => {
            let now = time::now();
            Outcome::Doc(crate::tokens_cmd::json(&quota_core::tokens::scan(now), now))
        }
        "tools_list" => Outcome::Doc(crate::tools_json(&quota_core::providers::detect::scan())),
        _ => return None,
    };
    Some(out)
}

fn refresh(only: Option<&[String]>) -> Outcome {
    let now = time::now();
    // Reuse a fresh snapshot that already covers what was asked for.
    if let Ok(snap) = quota_core::snapshot::Snapshot::load() {
        let covered = |id: &str| snap.provider(id).and_then(|p| p.attempted_at).is_some_and(|t| now - t < REFRESH_MIN_SECS);
        let all_fresh = match only {
            Some(ids) => ids.iter().all(|i| covered(i)),
            None => !snap.providers.is_empty() && snap.providers.iter().all(|p| covered(&p.id)),
        };
        if all_fresh {
            let scoped = output::scoped(&snap, only);
            let mut v = limits_cmd::doc(&scoped, now, &[]);
            v["refresh"] = json!({"outcome": "unchanged", "message": "Fetched less than a minute ago; not fetched again."});
            return Outcome::Doc(v);
        }
    }
    match limits_cmd::refresh_now(only, |_| {}) {
        Err(f) => tool_error(f),
        Ok(r) => {
            let scoped = output::scoped(&r.snap, only);
            let mut v = limits_cmd::doc(&scoped, r.now, &r.warnings);
            v["refresh"] = r.json();
            Outcome::Doc(v)
        }
    }
}

fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut e = json!({"code": code, "message": message});
    if let Some(d) = data {
        e["data"] = d;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": e})
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// The server keeps no state between requests: a legacy `initialize`
/// only picks the version echoed back, and modern requests stand alone.
#[derive(Default)]
pub struct Server;

impl Server {
    /// Handles one line from stdin. Returns the line to write, if any
    /// (notifications and client responses get none).
    pub fn handle_line(&self, line: &str) -> Option<Value> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return Some(rpc_error(Value::Null, -32700, "Parse error", None)),
        };
        let Some(obj) = msg.as_object() else {
            return Some(rpc_error(Value::Null, -32600, "Invalid request: expected one JSON-RPC object", None));
        };
        let id = obj.get("id").cloned();
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            // A response from the client (we send no requests) or junk.
            return match id {
                Some(id) if !obj.contains_key("result") && !obj.contains_key("error") => {
                    Some(rpc_error(id, -32600, "Invalid request: missing method", None))
                }
                _ => None,
            };
        };
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return id.map(|id| rpc_error(id, -32600, "Invalid request: jsonrpc must be \"2.0\"", None));
        }
        // Notifications (initialized, cancelled, ...) never get a reply.
        let id = match id {
            None => return None,
            Some(id @ (Value::String(_) | Value::Number(_))) => id,
            Some(_) => return Some(rpc_error(Value::Null, -32600, "Invalid request: id must be a string or number", None)),
        };
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        if !params.is_null() && !params.is_object() {
            return Some(rpc_error(id, -32602, "Invalid params: expected an object", None));
        }

        // The current revision: per-request version, no session.
        let meta = params.get("_meta");
        let modern = match meta.and_then(|m| m.get(META_VERSION)) {
            None => false,
            Some(v) => {
                let v = v.as_str().unwrap_or_default();
                if !MODERN.contains(&v) {
                    let supported: Vec<&str> = MODERN.iter().chain(LEGACY.iter()).copied().collect();
                    return Some(rpc_error(
                        id,
                        -32022,
                        "Unsupported protocol version",
                        Some(json!({"supported": supported, "requested": v})),
                    ));
                }
                if meta.and_then(|m| m.get(META_CAPS)).is_none_or(|c| !c.is_object()) {
                    return Some(rpc_error(id, -32602, &format!("Invalid params: _meta is missing {META_CAPS}"), None));
                }
                true
            }
        };
        let finish = |mut result: Value| -> Value {
            if modern {
                result["resultType"] = json!("complete");
                result["_meta"] = json!({"io.modelcontextprotocol/serverInfo": server_info()});
            }
            result
        };

        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
                let version = if LEGACY.contains(&asked) { asked } else { LEGACY[0] };
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": server_info(),
                    "instructions": INSTRUCTIONS,
                })
            }
            "ping" => json!({}),
            "server/discover" => json!({
                "resultType": "complete",
                "supportedVersions": MODERN,
                "capabilities": {"tools": {}},
                "instructions": INSTRUCTIONS,
                "_meta": {"io.modelcontextprotocol/serverInfo": server_info()},
                "ttlMs": 3_600_000,
                "cacheScope": "public",
            }),
            "tools/list" => {
                let mut r = json!({"tools": tools()});
                if modern {
                    r["ttlMs"] = json!(3_600_000);
                    r["cacheScope"] = json!("public");
                }
                finish(r)
            }
            "tools/call" => {
                let Some(name) = params.get("name").and_then(Value::as_str) else {
                    return Some(rpc_error(id, -32602, "Invalid params: name is required", None));
                };
                let empty = Map::new();
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => &empty,
                    Some(Value::Object(m)) => m,
                    Some(_) => return Some(rpc_error(id, -32602, "Invalid params: arguments must be an object", None)),
                };
                let Some(outcome) = call(name, args) else {
                    return Some(rpc_error(id, -32602, &format!("Unknown tool: {}", crate::style::clean(name)), None));
                };
                finish(match outcome {
                    Outcome::Doc(doc) => json!({
                        "content": [{"type": "text", "text": serde_json::to_string(&doc).unwrap_or_default()}],
                        "structuredContent": doc,
                        "isError": false,
                    }),
                    Outcome::Error(m) => json!({"content": [{"type": "text", "text": m}], "isError": true}),
                })
            }
            _ => return Some(rpc_error(id, -32601, "Method not found", None)),
        };
        Some(rpc_result(id, result))
    }
}

/// Serves until stdin closes. `err_tty`: a person started it by hand, so
/// say on stderr what it is waiting for.
pub fn run(err_tty: bool) -> ExitCode {
    if err_tty && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        eprintln!(
            "Quota's MCP server is waiting for an MCP client on stdin. Press Ctrl+C to stop.\n\n\
             To add it to Claude Code, run:\n    claude mcp add quota -- quota mcp"
        );
    }
    let server = Server;
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout().lock();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match input.read_until(b'\n', &mut buf) {
            Ok(0) => return ExitCode::SUCCESS,
            Ok(_) => {}
            Err(_) => return ExitCode::FAILURE,
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_line(line) {
            // serde_json never emits a raw newline inside a message.
            let text = serde_json::to_string(&reply).unwrap_or_default();
            if writeln!(out, "{text}").and_then(|_| out.flush()).is_err() {
                return ExitCode::SUCCESS;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(s: &Server, v: Value) -> Value {
        s.handle_line(&v.to_string()).expect("a reply")
    }

    #[test]
    fn legacy_handshake_and_listing() {
        let s = Server;
        let r = ask(&s, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}));
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["serverInfo"]["name"], "quota");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        assert!(r["result"].get("resultType").is_none(), "legacy results stay legacy");
        // A version we do not know: answer with our latest legacy one.
        let r = ask(&Server, json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}));
        assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert_eq!(ask(&s, json!({"jsonrpc":"2.0","id":"p","method":"ping"}))["result"], json!({}));
        let r = ask(&s, json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}));
        let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["limits_list", "limits_refresh", "tokens_list", "tools_list"]);
        for t in r["result"]["tools"].as_array().unwrap() {
            assert_eq!(t["inputSchema"]["type"], "object");
            assert!(t["name"].as_str().unwrap().chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }

    #[test]
    fn modern_requests_are_stateless() {
        let meta = json!({META_VERSION: "2026-07-28", META_CAPS: {}});
        let s = Server;
        let r = ask(&s, json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta": meta}}));
        assert_eq!(r["result"]["supportedVersions"], json!(["2026-07-28"]));
        let r = ask(&s, json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta": meta}}));
        assert_eq!(r["result"]["resultType"], "complete");
        assert_eq!(r["result"]["cacheScope"], "public");
        assert!(r["result"]["ttlMs"].is_number());
        let r = ask(&s, json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{"_meta":{META_VERSION:"1900-01-01", META_CAPS:{}}}}));
        assert_eq!(r["error"]["code"], -32022);
        assert_eq!(r["error"]["data"]["requested"], "1900-01-01");
        let r = ask(&s, json!({"jsonrpc":"2.0","id":4,"method":"tools/list","params":{"_meta":{META_VERSION:"2026-07-28"}}}));
        assert_eq!(r["error"]["code"], -32602, "client capabilities are required");
    }

    #[test]
    fn errors() {
        let s = Server;
        assert_eq!(s.handle_line("{not json").unwrap()["error"]["code"], -32700);
        assert_eq!(s.handle_line("[1]").unwrap()["error"]["code"], -32600);
        assert_eq!(ask(&s, json!({"jsonrpc":"2.0","id":1,"method":"nope"}))["error"]["code"], -32601);
        let r = ask(&s, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rm_rf"}}));
        assert_eq!((r["error"]["code"].as_i64(), r["error"]["message"].as_str()), (Some(-32602), Some("Unknown tool: rm_rf")));
        let r = ask(&s, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"limits_list","arguments":[]}}));
        assert_eq!(r["error"]["code"], -32602);
        // Bad arguments are a tool result the model can read, not a protocol error.
        let r = ask(&s, json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"limits_list","arguments":{"provider":["nope"]}}}));
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().starts_with("Unknown provider"));
        let r = ask(&s, json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"tools_list","arguments":{"x":1}}}));
        assert_eq!(r["result"]["isError"], true);
        // Client responses and notifications get no reply.
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#).is_none());
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#).is_none());
    }

    #[test]
    fn tools_list_returns_the_cli_document() {
        let s = Server;
        let r = ask(&s, json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tools_list"}}));
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(r["result"]["structuredContent"]["schema"], "quota.tools.v1");
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(serde_json::from_str::<Value>(text).unwrap(), r["result"]["structuredContent"]);
    }

    #[test]
    fn provider_argument() {
        let m = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(providers_arg(&m(json!({}))).unwrap(), None);
        assert_eq!(providers_arg(&m(json!({"provider": ["Claude", "claude"]}))).unwrap(), Some(vec!["claude".into()]));
        assert_eq!(providers_arg(&m(json!({"provider": "codex"}))).unwrap(), Some(vec!["codex".into()]));
        assert!(providers_arg(&m(json!({"provider": 3}))).is_err());
        assert!(providers_arg(&m(json!({"other": 1}))).is_err());
    }
}
