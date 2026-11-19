# Changelog

## 1.0.0 — Unreleased

First release of Quota, a Windows tray app and CLI for AI coding subscription limits and token use. Local only: no account, no server, no telemetry.

### Added

- CLI following Vercel's CLI design guidelines: `quota <group> <action>`, with `limits list` (the default; reads the local snapshot, no network), `refresh`, `status`, `tokens list`, `tools list`, `config path`, `config init` and `mcp`. Help at every level (`--help`, `help <command>`, `--help --json`). Results on stdout; progress, warnings and receipts on stderr. A table on a terminal, a tab-separated table with ISO times when piped, one JSON document with `--json` (`quota.limits.v2`, `quota.tokens.v2`, `quota.tools.v1`, `quota.status.v1`, `quota.config.v1`, camelCase keys). Errors give the command that fixes them; with `--json` they are one line on stderr with `status`, `reason` and `next`. Exit codes 0, 1, 2 and 130.
- `quota mcp`: a Model Context Protocol server (stdio) with `limits_list`, `limits_refresh`, `tokens_list` and `tools_list`, returning the same documents as `--json`. Serves protocol revision 2026-07-28 and the 2025-11-25 `initialize` handshake.
- `quota status`: one line about the most urgent limit, for shell prompts and Claude Code's status line.
- `quota tokens list` shows tokens used today, yesterday and in the last 30 days, split by tool, with a 14-day sparkline per tool. Read from Claude Code's and Codex's own session logs (counts and dates only), incrementally with a cache.
- `quota tools list` lists 25 AI coding tools, whether each is installed, and whether Quota meters it.
- `quota-tray` keeps one icon in the notification area. The ring is the percent left of the most urgent limit. Right-click has refresh, start with Windows, theme, config, data folder and quit.
- Popup built with Slint (software renderer, no WebView). It is created on click and destroyed on close. The home follows the OpenUsage menu: a Priority | All usage switch. Priority lists Pinned, Running out and Used up. All usage is one card per connected provider (logo, name, plan), with a thin bar per limit (blue, orange, red by how much is left), the percent or amount in a fixed column, and the reset time. Claude and ChatGPT add a Today line when today's logs have tokens. Providers that are not connected share one card. An empty home offers Add account; that page lists the five metered tools and how to sign in on this PC. Codex is labelled ChatGPT on the home and the sheet; the id stays `codex`. Settings saves start with Windows, notifications, theme, left-or-used, countdown-or-clock, the check interval and which providers to show, in place in config.toml; a change that would make the file unreadable is refused.
- When a limit is projected to run out before it resets, the CLI, the tooltip and the notification say "Stay under …" and "Runs out in …". The CLI summary still leads with the most urgent limit. The popup home does not.
- Providers: Claude Code, Codex, Cursor, GitHub Copilot and OpenRouter. A provider that fails shows its own short reason and does not affect the others. Copilot also finds the GitHub login that OpenCode stores.
- Burn budget on every limit: how fast the rest can be used and still last until reset (`0.5%/day`, `14%/h`, `50% in 12m`). Shown in the CLI, the tooltip, notifications and `--json` (`budget`). The popup home shows what is left and when it resets.
- Web-style polish: hand cursor, eased hover/press states, sliding tab indicator, footer tooltips, whole-row switches, fade/slide-in on open, Lucide icons, and Windows 11 rounded corners, shadow and themed border.
- Layout and flow: one 4px grid and one 16px edge on every page; 32px controls and 44/52px rows. The popup always reopens on Priority. Pinned rows come from `pinned` (`provider/Label`). Running out uses the higher notice threshold. Add account and Settings replace the home and have a back button. The footer is "Next update", a copy button on Priority, and "…". Clicking outside the menu closes it. A window that has ended shows "—" and "Window ended · refresh to update" on All usage and is left off Priority.
- Geist (Regular, Medium, SemiBold; SIL OFL 1.1) embedded. Dark theme is charcoal with a lighter card; light theme is a near-white window with white cards. Neutral buttons with one primary per page; bars are blue, orange and red. Theme: System, Light or Dark, from Settings, the "…" menu, `T`, the tray menu, or `theme` in config.toml (saved in place, comments kept). Show "left" or "used", and reset times as a countdown or a clock, from Settings (`usage_mode`, `reset_timers`).
- Keyboard: `R` / `F5` refresh, `T` theme, `←` `↑` Priority, `→` `↓` All usage, `Esc` closes Add account, Settings or the menu and then the popup. Windows *Animation effects* off disables animation.
- Color only on a terminal: `--no-color`, then a non-empty `NO_COLOR`, then `FORCE_COLOR`, then `TERM=dumb`. Text from providers and logs is cleaned of control and bidirectional characters before it is printed.
- Notifications at 30% and 10% left, and when pace says a limit runs out before it resets.
- Sponsor Quota: a link to GitHub Sponsors in the "…" menu and the README, and `.github/FUNDING.yml`.
- `quota-tray --preview <file.bmp>` renders the popup off-screen for design review; `--bench-popup` measures open time, memory and idle CPU.

### Fixed

- At 125%+ display scaling the popup was placed using 100% sizes and could run off the bottom of the screen. It now uses the monitor's DPI.
- A refresh while the popup was open resized it from its top edge, so a popup anchored to the bottom taskbar grew past the screen. It now re-anchors to where it was opened.
- Content taller than the screen was cut off with no way to reach it.
- A limit whose reset time had passed kept showing the old window's percentage, and could drive the summary and the tray icon. It now shows `—` and "Refresh to update", and is left out of urgency, notifications and the tooltip. `--json` adds `windowEnded`.
- Whole percentages could show one lower than they were: 89 of 100 used showed "10% left" (floating point made it 10.999…, and the display rounds down). Percent left is now rounded to 6 decimals first, in the CLI, the popup, the tooltip and notifications.

### Notes

- Cursor with an encrypted sign-in token reports unavailable. Quota does not decrypt another app's secrets.
- Copilot's quota fields follow the known response shape and were not exercised against an account that has a plan.
- 20 of the 25 detected tools are not metered. Their usage endpoints could not be verified on Windows, and Quota does not guess them.
- OpenRouter's free-model daily allowance is assumed to reset at 00:00 UTC.
