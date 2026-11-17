//! Terminal styling. Colour precedence, highest first (Vercel CLI
//! guidelines, "Color, width and platform"):
//!
//! 1. `--no-color`
//! 2. a non-empty `NO_COLOR`
//! 3. `FORCE_COLOR` (`0` means off; `CLICOLOR_FORCE` is read the same way)
//! 4. per stream: colour only on a terminal whose `TERM` is not `dumb`.
//!
//! Turning colour off removes only colour; text and layout stay the same,
//! and glyphs become words (`Warning:`, `Error:`). Only the 16 standard
//! ANSI colours are used, so the terminal's theme decides the shades.

use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Low,
    Critical,
    Muted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub color: bool,
}

fn env_on(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Decides colour for stdout (`err = false`) or stderr.
pub fn detect(no_color_flag: bool, err: bool) -> Style {
    let tty = if err { std::io::stderr().is_terminal() } else { std::io::stdout().is_terminal() };
    let color = decide(
        no_color_flag,
        std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
        env_on("FORCE_COLOR") || env_on("CLICOLOR_FORCE"),
        std::env::var("TERM").is_ok_and(|t| t == "dumb"),
        tty,
    );
    Style { color: color && enable_vt(err) }
}

/// Pure decision (unit-tested).
pub fn decide(flag_off: bool, no_color: bool, force: bool, dumb: bool, tty: bool) -> bool {
    if flag_off || no_color {
        return false;
    }
    if force {
        return true;
    }
    tty && !dumb
}

/// Turns on ANSI escape processing for the Windows console. Returns false
/// when the console refuses (very old conhost), so we fall back to plain.
#[cfg(windows)]
fn enable_vt(err: bool) -> bool {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE,
    };
    unsafe {
        let h = GetStdHandle(if err { STD_ERROR_HANDLE } else { STD_OUTPUT_HANDLE });
        let mut mode = 0;
        if GetConsoleMode(h, &mut mode) == 0 {
            // Not a console (e.g. mintty pipe forced on): assume ANSI works.
            return true;
        }
        mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 || SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_vt(_err: bool) -> bool {
    true
}

impl Style {
    pub const PLAIN: Style = Style { color: false };

    fn wrap(&self, code: &str, s: &str) -> String {
        if self.color && !s.is_empty() {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    /// The primary outcome, and command and option names in help.
    pub fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }

    /// Optional metadata only (ANSI bright black: readable on dark and
    /// light themes).
    pub fn muted(&self, s: &str) -> String {
        self.wrap("90", s)
    }

    pub fn tone(&self, t: Tone, s: &str) -> String {
        match t {
            Tone::Ok => self.wrap("32", s),
            Tone::Low => self.wrap("33", s),
            Tone::Critical => self.wrap("31", s),
            Tone::Muted => self.muted(s),
        }
    }

    pub fn tone_bold(&self, t: Tone, s: &str) -> String {
        match t {
            Tone::Ok => self.wrap("1;32", s),
            Tone::Low => self.wrap("1;33", s),
            Tone::Critical => self.wrap("1;31", s),
            Tone::Muted => self.bold(s),
        }
    }

    /// Help section headings (`Options:`), bold magenta.
    pub fn heading(&self, s: &str) -> String {
        self.wrap("1;35", s)
    }

    /// A complete command offered as an example or next step, cyan at
    /// normal weight. Stripping the colour gives back the exact command.
    pub fn command(&self, s: &str) -> String {
        self.wrap("36", s)
    }

    /// Gutter glyph for the one completed result. Plain output has no
    /// glyph; the sentence says what happened.
    pub fn done(&self) -> String {
        if self.color {
            self.wrap("32", "✓")
        } else {
            String::new()
        }
    }

    /// Gutter glyph for a warning, or `Warning:` in plain output.
    pub fn warn(&self) -> String {
        if self.color {
            self.wrap("33", "!")
        } else {
            "Warning:".into()
        }
    }

    /// Gutter glyph for a fatal error, or `Error:` in plain output.
    pub fn fail(&self) -> String {
        if self.color {
            self.wrap("31", "✗")
        } else {
            "Error:".into()
        }
    }
}

/// A line in the gutter: glyph in column 0, text from column 2. A word in
/// place of the glyph (plain output) is followed by one space.
pub fn gutter(glyph: &str, text: &str) -> String {
    if glyph.is_empty() {
        format!("  {text}")
    } else {
        format!("{glyph} {text}")
    }
}

/// Text from a provider, a log or a file is data: drop control, escape,
/// bidirectional and zero-width characters before it reaches a terminal.
/// Newlines and tabs become spaces so one value stays on one line.
pub fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' || c == '\t' || c == '\r' { ' ' } else { c })
        .filter(|c| {
            !c.is_control()
                && !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect()
}

/// Visible width, ignoring ANSI escapes.
pub fn width(s: &str) -> usize {
    let mut n = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c == 'm' {
                in_esc = false;
            }
        } else if c == '\x1b' {
            in_esc = true;
        } else {
            n += 1;
        }
    }
    n
}

/// Pads to a visible width (left-aligned).
pub fn pad(s: &str, w: usize) -> String {
    let n = width(s);
    if n >= w {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(w - n))
    }
}

/// Right-aligns to a visible width.
pub fn rpad(s: &str, w: usize) -> String {
    let n = width(s);
    if n >= w {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(w - n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_decision() {
        assert!(decide(false, false, false, false, true));
        assert!(!decide(false, false, false, false, false), "piped");
        assert!(!decide(true, false, true, false, true), "--no-color beats FORCE_COLOR");
        assert!(!decide(false, true, true, false, true), "NO_COLOR beats FORCE_COLOR");
        assert!(decide(false, false, true, false, false), "FORCE_COLOR when piped");
        assert!(!decide(false, false, false, true, true), "TERM=dumb");
    }

    #[test]
    fn widths_ignore_escapes() {
        let s = Style { color: true };
        let red = s.tone(Tone::Critical, "abc");
        assert_eq!(red, "\x1b[31mabc\x1b[0m");
        assert_eq!(width(&red), 3);
        assert_eq!(width(&pad(&red, 5)), 5);
        assert_eq!(rpad("ab", 4), "  ab");
        assert_eq!(Style::PLAIN.bold("x"), "x");
        assert_eq!(width("█░│"), 3);
    }

    #[test]
    fn glyphs_and_cleaning() {
        let c = Style { color: true };
        assert_eq!(width(&gutter(&c.warn(), "Low")), 5, "glyph in column 0, text from column 2");
        assert_eq!(gutter(&Style::PLAIN.warn(), "Low"), "Warning: Low");
        assert_eq!(gutter(&Style::PLAIN.done(), "Saved."), "  Saved.");
        assert_eq!(clean("a\u{1b}[31mb\u{202E}c\nd"), "a[31mbc d");
    }
}
