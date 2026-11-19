//! Checks on the design tokens in `ui/theme.slint`: text and meter colours
//! keep WCAG contrast in both themes, and no screen uses a colour literal.

use std::collections::HashMap;

const THEME: &str = include_str!("../ui/theme.slint");

/// `name -> (dark, light)` for every `out property <color> x: dark ? #a : #b;`
/// and every single-value `out property <color> x: #a;`.
fn tokens() -> HashMap<&'static str, ([u8; 3], [u8; 3])> {
    let mut map = HashMap::new();
    for line in THEME.lines() {
        let Some(rest) = line.trim().strip_prefix("out property <color> ") else { continue };
        let Some((name, value)) = rest.split_once(':') else { continue };
        let value = value.trim().trim_end_matches(';');
        let hexes: Vec<[u8; 3]> = value.split_whitespace().filter_map(hex).collect();
        match hexes.as_slice() {
            [d, l] => {
                map.insert(name.trim(), (*d, *l));
            }
            [c] => {
                map.insert(name.trim(), (*c, *c));
            }
            _ => {}
        }
    }
    map
}

fn hex(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn luminance(c: [u8; 3]) -> f64 {
    let ch = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
}

fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn theme_contrast() {
    let t = tokens();
    let get = |n: &str| *t.get(n).unwrap_or_else(|| panic!("token {n} missing"));
    for (fg, bg, min) in [
        ("text", "card", 4.5),
        ("text", "surface", 4.5),
        ("text-secondary", "card", 4.5),
        ("text-secondary", "surface", 4.5),
        // Orange and red figures are words, so they need text contrast.
        ("bar-low", "card", 4.5),
        ("bar-hot", "card", 4.5),
        ("primary-text", "primary", 4.5),
        // Bars and rings are graphics: 3:1 against their track.
        ("bar-ok", "track", 3.0),
        ("bar-low", "track", 3.0),
        ("bar-hot", "track", 3.0),
    ] {
        let (fd, fl) = get(fg);
        let (bd, bl) = get(bg);
        let (cd, cl) = (contrast(fd, bd), contrast(fl, bl));
        assert!(cd >= min, "dark {fg} on {bg}: {cd:.2} < {min}");
        assert!(cl >= min, "light {fg} on {bg}: {cl:.2} < {min}");
    }
}

fn slint_files() -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "slint") {
                out.push(p);
            }
        }
    }
    let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut files = Vec::new();
    walk(&ui, &mut files);
    assert!(!files.is_empty());
    files.retain(|f| !f.ends_with("theme.slint"));
    files
}

#[test]
fn no_colour_literals_outside_theme() {
    for f in &slint_files() {
        let text = std::fs::read_to_string(f).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let literal = code.split(|c: char| !c.is_ascii_alphanumeric() && c != '#').any(|w| hex(w).is_some());
            assert!(!literal, "{}:{}: colour literal; use a Theme token", f.display(), i + 1);
        }
    }
}

/// The grid lives in theme.slint. A page may only write 0px or 1px (a
/// hairline); every other size or gap is a Theme token. Window limits that
/// Rust overwrites (`in property` defaults) are not layout.
#[test]
fn no_size_literals_outside_theme() {
    for f in &slint_files() {
        let text = std::fs::read_to_string(f).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if code.trim_start().starts_with("in property") {
                continue;
            }
            for w in code.split(|c: char| !c.is_ascii_alphanumeric() && c != '.') {
                if let Some(n) = w.strip_suffix("px") {
                    let ok = n.parse::<f32>().is_ok_and(|v| v == 0.0 || v == 1.0) || n.parse::<f32>().is_err();
                    assert!(ok, "{}:{}: {w} is off the grid; use a Theme token", f.display(), i + 1);
                }
            }
        }
    }
}
