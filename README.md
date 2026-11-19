# Quota

**See how much of your AI coding limits you have left, and when they reset, from the Windows taskbar.**

Quota is a small Windows tray app and command-line tool for Claude Code,
Codex / ChatGPT, Cursor, GitHub Copilot and OpenRouter. It reads the sign-in
each tool already keeps on your PC. It runs entirely on your machine, with no
account, no server and no telemetry.

- [Features](#features)
- [Requirements](#requirements)
- [Installation](#installation)
- [Getting started](#getting-started)
- [Using the tray app](#using-the-tray-app)
- [Supported providers](#supported-providers)
- [Command-line interface](#command-line-interface)
- [MCP server for agents](#mcp-server-for-agents)
- [Configuration](#configuration)
- [Privacy and security](#privacy-and-security)
- [Building from source](#building-from-source)
- [Contributing a provider](#contributing-a-provider)
- [Performance](#performance)
- [Support the project](#support-the-project)
- [License and acknowledgements](#license-and-acknowledgements)

## Features

- **Always visible.** The tray icon is a ring showing what is left of your
  most urgent limit. Hover for the top three; click for the full picture.
- **Two views.** *Priority* shows only what needs attention: pinned limits,
  limits running out, and limits that are used up. *All usage* shows every
  limit for every provider, with a bar and its reset time.
- **Early warnings.** Windows notifications at 30% and 10% left, and when
  your current pace would use up a limit before it resets.
- **Today's tokens.** Token counts for Claude Code and Codex, read from their
  own local session logs.
- **Command line and agents.** `quota.exe` prints limits as a table, as
  tab-separated text or as versioned JSON. `quota mcp` lets an agent such as
  Claude Code check what is left before a long task.
- **Light on resources.** No WebView, Electron or browser engine. The popup
  window exists only while it is open; with it closed, Quota uses about
  0.2 MB of private memory and no measurable CPU.
- **Native feel.** Light and dark themes that follow Windows, Windows 11
  rounded corners and shadow, full keyboard control, and no animation when
  Windows *Animation effects* is off.

## Requirements

| | |
|---|---|
| Operating system | Windows 10 version 1809 or later, or Windows 11 (x64) |
| To build | Rust 1.82 or later (MSVC or GNU toolchain) |
| To show data | At least one supported tool signed in on this PC |

## Installation

Prebuilt downloads are not available yet. Build from source:

```bat
git clone https://github.com/Abiram08/quota.git
cd quota
cargo build --release
```

This produces two programs in `target\release\`:

| File | What it is |
|---|---|
| `quota-tray.exe` | The tray app. Run it, then turn on *Start with Windows* in Settings or the tray menu. |
| `quota.exe` | The command-line tool. Run `cargo install --path crates/cli` to put it on your `PATH`. |

## Getting started

Quota has no login of its own. To connect a tool, sign in to that tool on
this PC; Quota reads its sign-in read-only.

| Tool | How to connect |
|---|---|
| Claude Code | Run `claude` and sign in |
| Codex / ChatGPT | Run `codex login` |
| Cursor | Sign in inside Cursor |
| GitHub Copilot | Sign in to Copilot in your editor, or in OpenCode |
| OpenRouter | Set `OPENROUTER_API_KEY`, or sign in to OpenRouter in OpenCode |

Then click the Quota icon in the notification area. If nothing is connected
yet, choose **Add account** to see each tool's state and how to sign in, then
**Check again**.

## Using the tray app

### Priority

The popup opens on Priority, which lists only the limits that need attention:

| Section | Shows |
|---|---|
| **Pinned** | Limits you named in `pinned` in the config file, for example `claude/Weekly` |
| **Running out** | Limits at or below your higher notification threshold (30% by default), lowest first |
| **Used up** | Limits with nothing left |

Each row shows the limit, whose it is (for example *Claude · Work*), how much
is left and when it resets. Click a row to copy its reset time.

### All usage

One card per connected provider, showing its plan and every limit. Each limit
has a bar: blue while plenty is left, orange when it runs low, red when it is
almost gone. The percentage or amount (`$15.00`, `50 req`) is always written
beside the bar, so colour is never the only cue. Click a limit to copy its
reset time. Tools that are not connected are grouped in one card, with the
reason and a **How to connect** button.

### Settings

Open Settings from the **…** menu. Every change is saved immediately to
`config.toml`, and any comments you added to that file are kept.

| Setting | Options |
|---|---|
| Start with Windows | On or off |
| Notifications | On or off |
| Check every | 3, 5, 10 or 15 minutes |
| Theme | System, Light or Dark |
| Show | What is left, or what is used |
| Reset times | Countdown (*Resets in 3h 22m*) or clock time |
| Providers | Show or hide each provider |

### Keyboard shortcuts

| Key | Action |
|---|---|
| `←` or `↑` | Priority |
| `→` or `↓` | All usage |
| `R` or `F5` | Refresh now |
| `T` | Cycle the theme |
| `Esc` | Close the current page or menu, then the popup |

### Tray menu

Right-click the tray icon for **Refresh now**, **Start with Windows**,
**Theme**, **Open config file**, **Open data folder** and **Quit**.

## Supported providers

| Provider | Sign-in read from | Usage source | Status |
|---|---|---|---|
| Claude Code | `%USERPROFILE%\.claude\.credentials.json` | Claude Code's own cache, else `api.anthropic.com` | Verified |
| Codex / ChatGPT | `%USERPROFILE%\.codex\auth.json` | `chatgpt.com` usage endpoint | Verified (Free plan) |
| Cursor | Cursor's local state database (read-only) | `api2.cursor.sh` | Endpoint found; response not yet verified |
| GitHub Copilot | Copilot's or OpenCode's sign-in files | `api.github.com` | Endpoint verified; quota fields not yet verified |
| OpenRouter | `OPENROUTER_API_KEY`, or OpenCode's sign-in | `openrouter.ai` (documented API) | Verified (free tier) |

These usage endpoints are not public APIs and may change. Quota ignores
fields it does not know and tolerates missing ones. If a provider fails, only
that provider shows as *unavailable*, with a short reason; the others are
unaffected, and its last good data is kept and marked as older.

`quota tools list` also detects 20 other AI coding tools on your PC, such as
Gemini CLI, Windsurf and Cline, and marks them *not metered yet*. Quota adds
a provider only after its usage source has been verified on Windows; it
never guesses an endpoint or a response format.

## Command-line interface

`quota.exe` follows the shape `quota <group> <action> [options]`, modelled on
Vercel's [CLI design guidelines](https://vercel.com/design/cli).

```text
quota                        Same as quota limits list
quota limits list            Every limit, from the last saved check (no network)
quota limits list --provider claude,codex
quota refresh                Check every provider now, then list
quota status                 One line about the most urgent limit
quota tokens list            Tokens by tool, from local session logs
quota tools list             AI tools on this PC, and which are metered
quota config path            Where the config file is (quota config init creates it)
quota mcp                    Run the MCP server
quota help <command>         Help for a command (also <command> --help)
```

On a terminal, `quota` prints a summary and a table:

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
openrouter  Free models (daily)  ██████████ 100%  in 14h 59m  6.8%/h
```

**Status** is the burn budget: how fast you can use the rest and still reach
the reset. It reads *Out in …* when your current pace would run out first.

- **Scripts.** When piped, output is tab-separated with plain numbers and ISO
  8601 UTC times, ready for `findstr`, `sort` or a spreadsheet. `--json`
  prints one versioned document (`quota.limits.v2`).
- **Output streams.** Results go to stdout; progress, warnings and errors go
  to stderr. Errors name the command that fixes them.
- **Exit codes.** `0` success (including empty results), `1` failure, `2`
  usage error, `130` cancelled with Ctrl+C.
- **Colour.** Used only on a terminal. It is turned off by `--no-color`,
  `NO_COLOR` or `TERM=dumb`, and controlled by `FORCE_COLOR`.
- **Status line.** `quota status` prints one line for a shell prompt or an
  editor status line, for example in Claude Code's `settings.json`:

  ```json
  { "statusLine": { "type": "command", "command": "quota status" } }
  ```

`quota tokens list` reads only the token counts and dates from Claude Code
and Codex session logs, never message content. The first run reads every
log; later runs read only new lines.

## MCP server for agents

`quota mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io)
server over standard input and output. To add it to Claude Code:

```sh
claude mcp add quota -- quota mcp
```

For other clients, add this entry, using the full path to `quota.exe` if it
is not on your `PATH`:

```json
{ "mcpServers": { "quota": { "command": "quota", "args": ["mcp"] } } }
```

| Tool | Returns | Network |
|---|---|---|
| `limits_list` | Every limit, optionally for some providers | None |
| `limits_refresh` | Checks every provider now, then the same as `limits_list` | Each provider's own usage endpoint |
| `tokens_list` | Tokens by tool | None |
| `tools_list` | AI tools on this PC and which are metered | None |

The server supports MCP revision `2026-07-28` and the `initialize` handshake
of `2025-11-25` and earlier. It never changes a sign-in or any file other
than Quota's own data.

## Configuration

| Path | Contents |
|---|---|
| `%APPDATA%\quota\config.toml` | Settings |
| `%LOCALAPPDATA%\quota\snapshot.json` | The last known limits, shared by the tray app and the CLI |
| `%LOCALAPPDATA%\quota\notified.json` | Notifications already shown for the current window |
| `%LOCALAPPDATA%\quota\tokens-cache.json` | Daily token totals |

Settings changes are saved to `config.toml`. You can also edit it directly:

```toml
poll_interval_secs = 180              # 60 or more; failures back off up to 30 minutes
theme = "system"                      # system, light or dark
pinned = ["claude/Weekly"]            # shown under Pinned
usage_mode = "left"                   # left or used
reset_timers = "relative"             # relative (countdown) or absolute (clock)
enabled_providers = ["claude", "codex", "cursor", "copilot", "openrouter"]

[notifications]
enabled = true
thresholds_percent_left = [30, 10]
pace_warnings = true
```

Set `QUOTA_CONFIG_DIR` and `QUOTA_DATA_DIR` to use other folders.

## Privacy and security

- **Local only.** No telemetry, analytics or update checks. Quota's only
  network requests go to each provider's own usage endpoint, using the
  sign-in you already have.
- **Read-only sign-ins.** Quota never refreshes, rewrites or deletes another
  tool's tokens. If a sign-in has expired, open that tool once and it will
  renew its own.
- **No secrets stored or logged.** Quota keeps no credentials of its own, and
  tokens never appear in logs, errors or saved data.
- **No accounts.** There is nothing to sign up for.

## Building from source

```sh
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The project is a Cargo workspace:

| Crate | Contents |
|---|---|
| `crates/core` | Providers, limits, pace calculations, configuration and saved data |
| `crates/cli` | `quota.exe` and the MCP server |
| `crates/tray` | `quota-tray.exe`: the tray icon and the [Slint](https://slint.dev) popup |

`.cargo/config.toml` limits the build to two parallel jobs for machines with
little memory; delete it if you don't need that.

**GNU toolchain and paths with spaces.** The GNU `dlltool` fails when the
project path contains a space. Use `scripts\cargo` in place of `cargo`; it
takes the same arguments, and runs cargo through a junction (`<drive>:\quota`)
that it creates. The junction is only a pointer: deleting it deletes nothing.

**UI tools.** The popup's spacing, sizes and colours are defined only in
`crates/tray/ui/theme.slint` on a 4 px grid, and tests enforce this together
with WCAG contrast in both themes. To check the UI without a desktop session:

```bat
quota-tray --preview out.bmp [--light | --dark] [--tab priority | usage]
           [--settings | --connect | --welcome | --menu | --about] [--scale 2]
quota-tray --bench-popup out.txt [--tab priority | usage]
```

`--preview` renders the real popup to a bitmap, and `--bench-popup` measures
open time, memory and idle CPU. To keep test data out of your real profile,
point `QUOTA_DATA_DIR` and `QUOTA_CONFIG_DIR` at `.quota\` in the project
folder, which git ignores.

## Contributing a provider

1. **Find the real data source first.** Locate where the tool stores its
   sign-in on Windows and which endpoint the tool itself calls. Do not guess.
2. **Implement the provider** in `crates/core/src/providers/<id>.rs`, using
   the [`Provider`](crates/core/src/lib.rs) trait:

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

   Keep parsing in a pure `parse_*` function using the tolerant helpers in
   `json.rs`. Return a `ProviderError` with a short message that never
   contains a token, and never panic. Set `window_start` and `resets_at` when
   known, so pace warnings work.
3. **Register it** in `providers/mod.rs` (`ALL_IDS` and `by_id`).
4. **Add tests** with a sanitised fixture in `crates/core/tests/fixtures/`.

## Performance

Measured on the development machine with release builds (2026-11-19):

| Measure | Result |
|---|---|
| `quota-tray.exe` size | 12.4 MB, including the UI renderer and SQLite |
| `quota.exe` size | 2.3 MB |
| Popup first open | 526 ms (Priority), 596 ms (All usage) |
| Popup reopen | 77 ms (Priority), 83 ms (All usage) |
| Private memory, popup open | 3.5 MB (Priority), 4.7 MB (All usage) |
| Private memory, popup closed | 0.2 MB |
| Idle CPU, popup closed | 0 ms over 30 s |
| `quota tokens list`, cached | About 40 ms |

## Support the project

Quota is free, with no ads, accounts or paid tier. If it saves you from
hitting a limit in the middle of a task, you can support its development
through [GitHub Sponsors](https://github.com/sponsors/Abiram08), or choose
**Sponsor Quota** in the app's **…** menu. Quota opens the page in your
browser and makes no request itself.

## License and acknowledgements

Quota is released under the MIT License, as declared in `Cargo.toml`. A
`LICENSE` file has not been added yet.

- **[Slint](https://slint.dev) 1.18** draws the popup, used under the Slint
  Royalty-free License 2.0, which requires attribution: *About Quota* shows
  the Slint badge. The terms were checked in November 2026; forks should
  check <https://slint.dev/terms-and-conditions> again.
- **[Geist](https://github.com/vercel/geist-font)** 1.9.2 by Vercel is the
  interface font, under the SIL Open Font License 1.1. The licence ships in
  `crates/tray/ui/fonts/OFL.txt`.
- **[Lucide](https://lucide.dev)** 0.460.0 provides the interface icons,
  under the ISC License. The licence ships in
  `crates/tray/ui/icons/LUCIDE-LICENSE.txt`.
- **[simple-icons](https://simpleicons.org)** 15.22.0 provides the provider
  logos, under CC0 1.0.
- The popup layout follows **[OpenUsage](https://github.com/robinebers/openusage)**
  by Robin Ebers (MIT). No code or assets from OpenUsage are included.

Claude, ChatGPT, Codex, Cursor, GitHub Copilot, OpenRouter and other product
names and logos are trademarks of their respective owners. They are used
only to identify each provider, and their use does not imply endorsement.
