# Quota

How much of your AI coding subscription limits you have left, and when they
reset. A small Windows tray app and CLI, local only: no account, no server,
no telemetry.

The tray popup has two views. **Priority** lists pinned limits, limits that
are running out, and limits that are used up. **All usage** is one card per
provider, with a bar and a reset time. Pace and the burn budget (how fast you
can use what is left and still make the reset: `0.5%/day`, `14%/h`, or
`50% in 12m` when less than an hour remains) stay in the CLI, the tooltip,
notifications and JSON.

- **Tray** (`quota-tray.exe`): a ring for the percent left of the most urgent
  limit, a tooltip with the top 3, a popup on click, and notifications at
  30% / 10% left or when your pace runs out before the reset.
- **CLI** (`quota.exe`): limits, tokens and installed tools, as a table on a
  terminal, tab-separated when piped, or versioned JSON.
- **MCP server** (`quota mcp`): lets an agent such as Claude Code ask what is
  left before it starts a long task.
- **Light on resources.** Nothing UI-related exists until you click the tray
  icon; the popup window is destroyed when it closes. No WebView, Electron or
  Tauri.

## Install

Build from source (Rust 1.82 or later, Windows 10 1809 or later, x64):

```sh
cargo build --release
cargo install --path crates/cli   # puts quota.exe on your PATH (~\.cargo\bin)
```

Run `target\release\quota-tray.exe` for the tray. There are no prebuilt
downloads yet.

## Connecting tools

Quota has **no account and no server**, so there is nothing to sign up for.
You connect a tool by signing in to that tool on this PC, and Quota reads
that login read-only:

| Tool | How to connect |
|---|---|
| Claude Code | Run `claude` and sign in |
| Codex | Run `codex login` |
| Cursor | Sign in inside Cursor |
| GitHub Copilot | Sign in to Copilot in your editor (or in OpenCode) |
| OpenRouter | Set `OPENROUTER_API_KEY`, or sign in to OpenRouter in OpenCode |

## The popup

The popup opens on **Priority**. A switch at the top moves to **All usage**.
Codex is labelled ChatGPT here; the provider id in config, snapshots and the
CLI stays `codex`.

**Priority** has three sections:

- **Pinned** — limits named in `pinned` (`provider/Label`, for example `claude/Weekly`).
- **Running out** — percent left at or under the higher notice threshold (30 unless you changed it), lowest first.
- **Used up** — nothing left. The row is dim and says "Used up".

A healthy limit that is not pinned is only on All usage. Each row is the provider logo in a ring, the limit name, whose it is ("Claude · Work"), how much is left, and when it resets. Click a row to copy the reset time. When nothing needs attention, Priority says so and offers *Show all usage*.

**All usage** is one card per connected provider: logo, name, and the plan on the right. Each limit is a name and its reset time ("Resets in 3h 22m"; click to copy), over a thin bar with the figure in a fixed column on the right. The bar is blue when plenty is left, orange under the higher threshold, red under the lower one. The figure is the same percent the Priority row calls "left" (or "used", from Settings). Other units show the amount (`$15.00`, `50 req`) instead. Claude and ChatGPT add a Today line when today's session logs contain tokens. Providers that are not connected share one *Not connected* card with their reasons and a *How to connect* button. A window that has ended shows "—" and "Window ended · refresh to update".

With nothing connected, the home says *Nothing connected yet* with an *Add account* button. **Add account** lists the five tools Quota can meter, with each one's state. Quota has no login of its own: opening a row shows how to sign in to that tool on this PC, and *Check again* re-reads the sign-ins.

The footer says when the next check is. Priority has a copy button for the summary. "…" opens Refresh, Copy summary, Add account, Settings, Theme, Sponsor Quota, About Quota and Quit.

| Page | What it answers |
|---|---|
| **Priority** | Pinned, running out, and used up. |
| **All usage** | Every limit, by provider, plus today's tokens for Claude and ChatGPT. |
| **Add account** | The five metered tools, whether each is connected, and how to sign in. |
| **Settings** | Start with Windows, notifications, how often to check (3, 5, 10 or 15 min), theme, show left or used, reset times as a countdown or a clock, and which providers to show. Each change is saved straight into config.toml, comments kept. |

There is no Quota login. Each tool keeps its own sign-in on your PC; Quota
reads it (read-only) and calls that tool's own usage endpoint.

Keys: `←` `↑` open Priority, `→` `↓` open All usage, `R` or `F5` refreshes,
`T` cycles the theme, `Esc` closes Add account, Settings or the menu, and
then the popup.

Right-click the tray icon for *Refresh now*, *Start with Windows* (HKCU `Run`
key), *Theme*, *Open config file*, *Open data folder* and *Quit*.

### Look

- **One grid.** Everything sits on a 4px grid defined in `crates/tray/ui/theme.slint`: 8px between a label and what it labels, 16px between items and at the popup's edge, 24px between sections. Every page shares one left and one right edge; rows that highlight on hover reach 8px past it so their text stays on it.
- **Hit targets.** Controls are 32px tall and rows 44px (52px with a second line), the same on every page. Whole rows toggle in Settings, not only the switch.
- **Colour means data.** Surfaces and buttons are neutral, with one filled primary action per page. Bars are blue, orange or red by how much is left, and the figure is written beside the bar, so colour is never the only cue. Logos carry their brand colour (a near-black brand uses the text colour).
- **Type.** **Geist** (SIL OFL 1.1, embedded in the exe; see THIRD_PARTY.md) in four sizes (12, 14, 16, 24) and three weights. Labels are sentence case.
- Text contrast is at least 4.5:1 in both themes, and bar fills at least 3:1 against the track, checked by a test (`crates/tray/src/theme_tests.rs`). The same test fails if a colour is written anywhere but `theme.slint`.
- The popup always reopens on Priority. Add account and Settings replace the home and have a back button. On Windows 11 the popup has rounded corners, the system shadow and a border in the theme colour.
- Theme: **System** (follows Windows), **Light** or **Dark**, from Settings, the "…" menu, `T`, the tray menu, or `theme` in config.toml (saved in place, comments kept). Windows *Animation effects* off turns every animation off.
- Icons: Lucide outline icons (ISC) for the interface. Provider logos are single-colour SVGs from simple-icons; they are trademarks used only to identify the provider, with sources and licences in [THIRD_PARTY.md](THIRD_PARTY.md).

## Providers

| Provider | Credentials (read-only) | Usage source | Status |
|---|---|---|---|
| Claude Code | `%USERPROFILE%\.claude\.credentials.json` (`CLAUDE_CONFIG_DIR`) | Claude Code's own cache in `~\.claude.json`, else `GET api.anthropic.com/api/oauth/usage` | verified |
| Codex / ChatGPT | `%USERPROFILE%\.codex\auth.json` (`CODEX_HOME`) | `GET chatgpt.com/backend-api/wham/usage` | verified (Free plan) |
| Cursor | `%APPDATA%\Cursor\User\globalStorage\state.vscdb` (SQLite, read-only) | `api2.cursor.sh` `DashboardService/GetCurrentPeriodUsage` | endpoint found, **response unverified**; installs with token encryption at rest show "unavailable" |
| GitHub Copilot | `%LOCALAPPDATA%\github-copilot\apps.json` / `hosts.json`, else OpenCode's `auth.json` (`github-copilot`) | `GET api.github.com/copilot_internal/user` | endpoint verified with both token sources, **quota fields unverified** (test account had no plan) |
| OpenRouter | `OPENROUTER_API_KEY`, else OpenCode's `auth.json` (`openrouter.key`) | `GET openrouter.ai/api/v1/key` and `/api/v1/credits` (documented) | verified (free tier); free-model daily reset assumed 00:00 UTC |

### The other AI tools

`quota tools list` checks 25 AI coding tools (Claude Code, Codex, Cursor, Copilot,
OpenRouter, Gemini CLI, Antigravity, Windsurf, Zed, Factory Droid, OpenCode,
Grok CLI, Kiro, Amazon Q, Cline, Roo Code, Kilo Code, Continue, Augment,
Tabnine, Trae, Qwen Code, Amp, Zencoder, Blackbox) by looking for their own
files. It sends nothing over the network. Five are metered. The rest are
listed as **not metered yet** by `quota tools list`, because their usage endpoints could not be verified on Windows. Quota
does not guess endpoints or response shapes. On the dev machine, Gemini CLI's
login had expired (and Quota never refreshes tokens), and Grok CLI's usage
call is not a plain HTTP endpoint.

All endpoints are undocumented and may change. Parsing ignores unknown fields
and tolerates missing ones. A failing provider shows as *unavailable* with a
short reason (or keeps its last data marked *stale*).

**Quota never refreshes, rewrites, or deletes another tool's tokens, and
never logs them.** If a token has expired, run that tool once so it refreshes
its own login.

## Usage

The CLI follows Vercel's [CLI design guidelines](https://vercel.com/design/cli):
`quota <group> <action> [options]`, results on stdout, everything else on
stderr, one JSON document with `--json`, and exit codes 0 (done, including
empty results), 1 (failed), 2 (usage error) and 130 (Ctrl+C).

```text
quota                         # same as quota limits list
quota limits list             # every limit, from the saved snapshot (no network)
quota limits list --provider claude,codex
quota refresh                 # fetch now from each provider, save, then list
quota status                  # one line about the most urgent limit
quota tokens list             # tokens by tool, from local session logs
quota tools list              # which of 25 AI tools are on this PC, and which are metered
quota config path             # where config.toml is (config init creates it)
quota mcp                     # MCP server for agents (see below)
quota help <command>          # also: quota <command> --help, --help --json
```

On a terminal, `quota` prints when the data was updated, one summary, and a
table:

```text
$ quota
  Updated         now

  Claude runs out in 1h 8m
  53% of Session (5h) left, and it resets in 4h 2m. Stay under 13%/h to make it
  to the reset.

Provider    Limit                Left             Resets      Status
claude      Session (5h)         █████░░░░░  53%  in 4h 2m    Out in 1h 8m
claude      Weekly               █░░░░░░░░░   8%  in 1d 10h   Out in 11h 40m
codex       Monthly              █░░░░░░░░░  11%  in 18d      Out in 1d 9h
cursor      —                    —                —           Not connected: Sign-in token is encrypted; Quota can't read it
openrouter  Free models (daily)  ██████████ 100%  in 14h 59m  6.8%/h
```

Bars show the amount **left**: a full bar means plenty. Status is the burn
budget (how fast you can use the rest and still make it to the reset), or
"Out in …" when your current pace runs out first. A window whose reset time
has passed shows `—` and "Refresh to update", not the old window's numbers.

Piped, the same command prints a tab-separated table with a header row,
plain numbers and ISO 8601 UTC times, so it works with `findstr`, `sort` or
a spreadsheet. `--json` prints one document (`quota.limits.v2`, camelCase
keys); the MCP tools return the same documents.

```text
$ quota | findstr codex
codex	Monthly	11	2026-12-05T22:52:38Z	Out in 1d 9h
```

Warnings, progress and receipts go to stderr: "This data is from 3h ago"
with the command that updates it, "Refreshing 5 providers…", "✓ Refreshed 3
providers in 1.2s". Providers that are not connected are skipped, not failed;
`quota refresh` exits 1 only when a connected provider failed. Errors name
what failed and give the command that fixes it:

```text
$ quota claude
✗ Unknown command claude for quota.

To list one provider's limits, run:
    quota limits list --provider claude
```

With `--json`, a failure writes nothing to stdout and one line to stderr:
`{"status":"error","reason":"unknown_command","message":"…","next":[…]}`.

`quota status` prints one line, for a shell prompt or a status line, e.g.
in Claude Code's `settings.json`:

```json
{ "statusLine": { "type": "command", "command": "quota status" } }
```

`quota tokens list` counts what Claude Code (`~\.claude\projects`) and Codex
(`~\.codex\sessions`) wrote to their own session logs. Counts include cached
input tokens. Only the counts and dates are read, never message content. The
first run reads every log (about 2 s for 360 MB); after that only new lines
are read (about 40 ms), using `%LOCALAPPDATA%\quota\tokens-cache.json`.

Color only on a terminal. Precedence: `--no-color`, then a non-empty
`NO_COLOR`, then `FORCE_COLOR` (`0` means off), then `TERM=dumb`. Without
color, glyphs become words (`Warning:`, `Error:`) and nothing else changes.
Only the 16 standard ANSI colors are used, so your terminal theme applies.

## MCP server

`quota mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io)
server on stdin and stdout, so an agent can check your limits before a long
task, or tell you which tool still has room. Add it to Claude Code:

```sh
claude mcp add quota -- quota mcp
```

Other clients take the usual entry (use the full path to `quota.exe` if it
is not on your PATH):

```json
{ "mcpServers": { "quota": { "command": "quota", "args": ["mcp"] } } }
```

| Tool | What it returns | Network |
|---|---|---|
| `limits_list` | every limit (`quota.limits.v2`), optionally for some providers | none |
| `limits_refresh` | fetches, saves, then the same document plus a refresh summary; reuses data less than a minute old | each provider's own usage endpoint |
| `tokens_list` | tokens by tool (`quota.tokens.v2`) | none |
| `tools_list` | AI tools on this PC and which are metered (`quota.tools.v1`) | none |

Each result has the JSON in a text block and in `structuredContent`. The
server speaks the current protocol revision (`2026-07-28`, stateless, with
`server/discover`) and the `initialize` handshake of `2025-11-25` and
earlier, which most clients still use. It writes nothing but protocol
messages, logs nothing, and has no tool that changes a sign-in or a file
other than Quota's own snapshot.

Design review without a desktop: `quota-tray --preview out.bmp [--light|--dark]
[--tab priority|usage] [--connect|--welcome|--settings|--menu|--about]
[--scale 2]` renders the real popup off-screen to a bitmap.
`quota-tray --bench-popup out.txt` measures popup open time, memory and idle CPU.

## Files

| Path | What |
|---|---|
| `%APPDATA%\quota\config.toml` | poll interval, enabled providers, notification thresholds |
| `%LOCALAPPDATA%\quota\snapshot.json` | last known limits (`quota.snapshot.v1`), shared by tray and CLI |
| `%LOCALAPPDATA%\quota\notified.json` | which notifications already fired this window |
| `%LOCALAPPDATA%\quota\tokens-cache.json` | per-day token totals and how far each session log was read |

```toml
poll_interval_secs = 180              # min 60; failures back off up to 30 min
theme = "system"                      # system, light or dark
pinned = ["claude/Weekly"]            # shown under Pinned on Priority
usage_mode = "left"                   # left or used
reset_timers = "relative"             # relative (countdown) or absolute (clock)
enabled_providers = ["claude", "codex", "cursor", "copilot", "openrouter"]

[notifications]
enabled = true
thresholds_percent_left = [30, 10]
pace_warnings = true
```

`QUOTA_DATA_DIR` and `QUOTA_CONFIG_DIR` override the folders above. For
development, point both at `.quota\` in the project (it is in `.gitignore`)
so everything Quota writes stays inside the checkout:

```bat
setx QUOTA_DATA_DIR "%CD%\.quota"
setx QUOTA_CONFIG_DIR "%CD%\.quota"
```

## Sponsors

Quota is free and has no ads, accounts or paid tier. If it saves you from
hitting a limit mid-task, you can support its development:

- **GitHub Sponsors:** <https://github.com/sponsors/Abiram08>
- In the app: **… menu, Sponsor Quota** opens the same page in your browser.
  Quota itself makes no request; it only hands the link to Windows.

Sponsors who want to be listed here will appear below.

<!-- sponsors -->
_No sponsors yet. Be the first._
<!-- /sponsors -->

## License

Quota's own code: MIT, as set in `Cargo.toml` (the `LICENSE` file is not
added yet).

The popup is built with [Slint](https://slint.dev) 1.18, which is offered
under three licenses: GPL-3.0-only, the Slint Royalty-free License 2.0, or a
paid commercial license. Quota uses the **Royalty-free License 2.0**, which
allows free use in desktop apps (not embedded devices) on one condition:
attribution, either the `AboutSlint` widget in the app's About screen or a
Slint badge on the app's public web page. *About Quota* (in the "…" menu)
shows it. Terms checked 2026-11; if you fork Quota, read
<https://slint.dev/terms-and-conditions> again.

Other dependencies and the provider logos are listed with their licenses in
[THIRD_PARTY.md](THIRD_PARTY.md).

## Privacy

Everything stays local. No telemetry, and no update check. The only network
requests go to each provider's own usage endpoint, using the login you
already have. Quota stores no credentials of its own.

## Building

```sh
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Works with the `x86_64-pc-windows-msvc` or `-gnu` toolchain. Binaries land in
`target/release/` (`quota.exe`, `quota-tray.exe`). `.cargo/config.toml` sets
`jobs = 2` for low-RAM machines; delete it if that does not apply to you.

With the GNU toolchain, the project path must not contain a space: GNU
`dlltool` fails on it. If yours does, use `scripts\cargo` in place of
`cargo` (it takes the same arguments):

```bat
scripts\cargo build --release
scripts\cargo test --workspace
```

It runs cargo through a junction `<drive>:\quota` that points at the project,
and creates that junction when it is missing. The junction is only a pointer:
deleting it deletes nothing, and the build output still lands in this
project's `target/`. When the path has no space, the script just runs cargo.

## How to add a provider

1. Find the real data first: where the tool stores its login on Windows
   (files under `%USERPROFILE%`, `%APPDATA%`, `%LOCALAPPDATA%`, Credential
   Manager, SQLite) and which endpoint the tool itself calls. Do not guess.
2. Create `crates/core/src/providers/<id>.rs` implementing
   [`Provider`](crates/core/src/lib.rs):

   ```rust
   pub struct Foo;
   impl Provider for Foo {
       fn id(&self) -> &'static str { "foo" }
       fn name(&self) -> &'static str { "Foo" }
       fn fetch(&self) -> Result<Vec<Limit>, ProviderError> {
           let creds = read_json(&path, "not signed in to Foo")?;   // read-only
           let body = crate::http::get_json(URL, &[("Authorization", &bearer)])?;
           parse_usage(&body)                                      // pure, unit-tested
       }
   }
   ```

   - Keep parsing in a pure `parse_*` function over `serde_json::Value` and use
     the tolerant helpers in `json.rs` (`num_any`, `ts_any`, `at`, …).
   - Return `ProviderError::{NotConfigured, Auth, Network, Parse, Unavailable}`
     with a short message that never contains a token. Do not panic.
   - Set `window_start` and `resets_at` when known so pace math, the burn
     budget and "Out in …" work.
3. Register it in `providers/mod.rs` (`ALL_IDS` and `by_id`).
4. Add tests with a sanitised fixture in `crates/core/tests/fixtures/`.

## Measurements

Release binaries from this machine, rebuilt on 2026-11-19. Popup numbers come
from `quota-tray --bench-popup <out.txt> --tab priority|usage`: the popup is
opened, measured, closed and measured again with no tray icon.

| | |
|---|---|
| `quota.exe` | 2,396,672 bytes |
| `quota-tray.exe` | 12,986,880 bytes (Slint's software renderer and bundled SQLite) |
| Popup, first open | 526 ms (Priority), 596 ms (All usage) |
| Popup, reopen | 77 ms (Priority), 83 ms (All usage) |
| Private memory, popup open | 3.5 MB (Priority), 4.7 MB (All usage) |
| Private memory, popup closed | 0.2 MB |
| Idle CPU, popup closed | 0 ms over 30 s |
| `quota tokens list`, cached | about 40 ms (first full scan of 360 MB of logs: 2.3 s) |
