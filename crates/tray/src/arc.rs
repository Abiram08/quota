//! Ring geometry for the Slint `Path` element: SVG path commands in a
//! 100x100 viewbox. Pure and unit-tested; the .slint side only draws.

/// Radius in viewbox units for a ring of `size_px` with a `stroke_px` line,
/// so the stroke stays inside the element.
///
/// Radius so a `stroke_px` line stays inside a `size_px` ring.
pub fn radius(size_px: f32, stroke_px: f32) -> f32 {
    let stroke_vb = if size_px > 0.0 { stroke_px / size_px * 100.0 } else { 0.0 };
    (50.0 - stroke_vb / 2.0).max(1.0)
}

fn fmt(v: f32) -> String {
    // Two decimals is plenty at popup sizes and keeps the strings short.
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A full circle (the track behind a priority ring).
pub fn circle(r: f32) -> String {
    let top = fmt(50.0 - r);
    let bottom = fmt(50.0 + r);
    let r = fmt(r);
    format!("M 50 {top} A {r} {r} 0 1 1 50 {bottom} A {r} {r} 0 1 1 50 {top}")
}

/// Arc for `fraction` (0..=1) of a full turn, from 12 o'clock clockwise.
/// Empty for 0; a full circle at 1. Drawn as the priority ring.
pub fn arc(fraction: f64, r: f32) -> String {
    let f = fraction.clamp(0.0, 1.0);
    if f <= 0.0005 {
        return String::new();
    }
    if f >= 0.9995 {
        return circle(r);
    }
    let a = f * std::f64::consts::TAU;
    let (x, y) = (50.0 + r as f64 * a.sin(), 50.0 - r as f64 * a.cos());
    let large = if f > 0.5 { 1 } else { 0 };
    let rs = fmt(r);
    format!("M 50 {} A {rs} {rs} 0 {large} 1 {} {}", fmt(50.0 - r), fmt(x as f32), fmt(y as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_keeps_stroke_inside() {
        // 40 px ring, 4 px stroke: stroke is 10 viewbox units, radius 45.
        assert!((radius(40.0, 4.0) - 45.0).abs() < 1e-4);
        assert_eq!(radius(0.0, 4.0), 50.0);
    }

    #[test]
    fn quarter_half_and_three_quarters() {
        assert_eq!(arc(0.25, 45.0), "M 50 5 A 45 45 0 0 1 95 50");
        assert_eq!(arc(0.5, 45.0), "M 50 5 A 45 45 0 0 1 50 95");
        // Past halfway the large-arc flag flips.
        assert_eq!(arc(0.75, 45.0), "M 50 5 A 45 45 0 1 1 5 50");
    }

    #[test]
    fn empty_and_full() {
        assert_eq!(arc(0.0, 45.0), "");
        assert_eq!(arc(-1.0, 45.0), "");
        assert_eq!(arc(1.0, 45.0), circle(45.0));
        assert_eq!(circle(45.0), "M 50 5 A 45 45 0 1 1 50 95 A 45 45 0 1 1 50 5");
    }
}
