# Third-party material

## Provider logos

Quota shows a provider's logo only to identify that provider. All names and
logos are trademarks of their respective owners; their use here does not
imply endorsement. No logo is fetched over the network: the unmodified SVG
files are in `crates/tray/ui/icons/`, embedded in the binary at build time
and tinted to the theme colour (Slint `colorize`).

The glyphs come from **simple-icons 15.22.0** (<https://simpleicons.org>),
whose icon collection is released under **CC0 1.0 Universal**
(<https://github.com/simple-icons/simple-icons/blob/develop/LICENSE.md>).
Per-icon source and licence fields from simple-icons'
`data/simple-icons.json`:

| Provider in Quota | simple-icons slug | Upstream source (per simple-icons) | Licence | Brand guidelines |
|---|---|---|---|---|
| Claude Code | `claude` | https://claude.ai | CC0 1.0 (simple-icons) | — |
| Codex / ChatGPT | `openai` | https://openai.com/brand | CC0 1.0 (simple-icons) | https://openai.com/brand |
| Cursor | `cursor` | https://cursor.com/brand | CC0 1.0 (simple-icons) | — |
| GitHub Copilot | `githubcopilot` | https://primer.style/foundations/icons/copilot-24 | MIT (GitHub Primer Octicons), CC0 1.0 (simple-icons) | — |
| OpenRouter | `openrouter` | https://openrouter.ai | CC0 1.0 (simple-icons) | — |

### Providers without a licensed glyph

simple-icons 15.22.0 has no icon for **Antigravity, Grok / xAI, OpenCode,
Z.ai or Devin** (checked by slug: `antigravity`, `grok`, `xai`, `opencode`,
`zai`, `zhipu`, `devin`, `cognition`; all returned 404). simple-icons' `x`
icon is X (formerly Twitter), not Grok, so it is not used. These providers
use Quota's own fallback: the provider's initial in the ring, drawn as text.
No icon or asset was taken from OpenUsage or any other app.

## Layout reference

The popup's layout (a Priority | All usage switch over provider cards with
one bar per limit) follows **OpenUsage** by Robin Ebers
(<https://github.com/robinebers/openusage>, MIT License, Copyright (c)
Robin Ebers). Quota is written from scratch in Rust and Slint: no code,
text, icon or other asset from OpenUsage is included. The About sheet
names OpenUsage.

## Interface icons

The footer, back and chevron icons (`lucide-*.svg` in
`crates/tray/ui/icons/`) are from **Lucide** (`lucide-static` 0.460.0,
<https://lucide.dev>, fetched from
<https://unpkg.com/lucide-static@0.460.0/icons/>), licensed under the
**ISC License**. The files are unmodified; the licence text is in
`crates/tray/ui/icons/LUCIDE-LICENSE.txt`.

| File | Lucide icon |
|---|---|
| `lucide-copy.svg` | `copy` |
| `lucide-ellipsis.svg` | `ellipsis` (more) |
| `lucide-chevron-left.svg` | `chevron-left` (back) |
| `lucide-chevron-right.svg` | `chevron-right` (a closed row) |
| `lucide-chevron-down.svg` | `chevron-down` (an open row) |

## Fonts

The popup uses **Geist** by Vercel and the Geist Project
Authors, version **1.9.2**, from the official release
<https://github.com/vercel/geist-font/releases/tag/v1.9.2>
(`geist-font-v1.9.2.zip`). They are licensed under the **SIL Open Font
License 1.1**; the licence text ships next to the fonts in
`crates/tray/ui/fonts/OFL.txt`.

The unmodified static TTF files are in `crates/tray/ui/fonts/` and are
embedded in `quota-tray.exe` at build time; no font is downloaded.

| File | Family | Weight |
|---|---|---|
| `Geist-Regular.ttf` | Geist | 400 |
| `Geist-Medium.ttf` | Geist | 500 |
| `Geist-SemiBold.ttf` | Geist | 600 |

The OFL allows bundling the fonts with software as long as the fonts are
not sold on their own and the licence goes with them. The copyright notice
in `OFL.txt` declares no Reserved Font Name, and Quota ships the files
unmodified.

## Quota's own artwork

The tray ring icon (`crates/tray/src/icon.rs`) and the usage rings on the
Priority page (`crates/tray/src/arc.rs`) are drawn by Quota's own code.
