//! Tray icons: a ring whose coloured arc is the percent *left* of the most
//! urgent limit. Rendered once per (level, colour, theme) at the tray's
//! pixel size and cached — 11 levels x 3 colours x 2 themes at most.

use std::collections::HashMap;
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tone {
    Ok,
    Low,
    Critical,
    NoData,
}

impl Tone {
    pub fn for_percent_left(left: Option<f64>) -> Tone {
        match left {
            None => Tone::NoData,
            Some(l) if l <= 10.0 => Tone::Critical,
            Some(l) if l <= 30.0 => Tone::Low,
            Some(_) => Tone::Ok,
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            // Cursor semantic colors, lifted enough to read on a dark taskbar.
            Tone::Ok => [31, 160, 116],
            Tone::Low => [212, 146, 58],
            Tone::Critical => [207, 60, 96],
            Tone::NoData => [140, 136, 126],
        }
    }
}

/// 0..=10 (tenths left). Any non-zero amount left shows at least one step.
pub fn level(percent_left: Option<f64>) -> u8 {
    match percent_left {
        None => 0,
        Some(p) if p <= 0.0 => 0,
        Some(p) => ((p / 10.0).round() as u8).clamp(1, 10),
    }
}

/// Straight-alpha BGRA pixels, top-down rows, `size * size * 4` bytes.
pub fn render(size: usize, level: u8, tone: Tone, light_taskbar: bool) -> Vec<u8> {
    const SS: usize = 4; // 4x4 supersampling
    let fraction = level.min(10) as f64 / 10.0;
    let track: [u8; 3] = if light_taskbar { [70, 70, 70] } else { [200, 200, 200] };
    let color = tone.rgb();
    let c = size as f64 / 2.0;
    let outer = c - 0.5;
    let inner = outer - (size as f64 * 0.26).max(2.5);
    let dot = size as f64 * 0.16;
    let mut px = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (mut arc, mut trk, mut ctr) = (0u32, 0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = x as f64 + (sx as f64 + 0.5) / SS as f64 - c;
                    let fy = y as f64 + (sy as f64 + 0.5) / SS as f64 - c;
                    let r = (fx * fx + fy * fy).sqrt();
                    if r <= outer && r >= inner {
                        // 0 at 12 o'clock, increasing clockwise.
                        let mut a = fx.atan2(-fy) / std::f64::consts::TAU;
                        if a < 0.0 {
                            a += 1.0;
                        }
                        if a < fraction {
                            arc += 1;
                        } else {
                            trk += 1;
                        }
                    } else if level == 0 && tone == Tone::Critical && r <= dot {
                        ctr += 1;
                    }
                }
            }
            let n = (SS * SS) as f64;
            let (wa, wt, wc) = (arc as f64 / n, trk as f64 * 0.45 / n, ctr as f64 / n);
            let alpha = wa + wt + wc;
            let i = (y * size + x) * 4;
            if alpha > 0.0 {
                let mix = |ch: usize| {
                    ((color[ch] as f64 * (wa + wc) + track[ch] as f64 * wt) / alpha).round() as u8
                };
                px[i] = mix(2);
                px[i + 1] = mix(1);
                px[i + 2] = mix(0);
                px[i + 3] = (alpha.min(1.0) * 255.0).round() as u8;
            }
        }
    }
    px
}

fn create_hicon(size: usize, bgra: &[u8]) -> HICON {
    unsafe {
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let dc = GetDC(std::ptr::null_mut());
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        ReleaseDC(std::ptr::null_mut(), dc);
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: the DIB section holds exactly size*size 32-bit pixels.
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, size * size * 4);
        let mask_bytes = vec![0u8; size.div_ceil(16) * 2 * size];
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, mask_bytes.as_ptr() as *const _);
        let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

/// Cache of rendered icons; destroys them on drop.
pub struct IconCache {
    size: usize,
    icons: HashMap<(u8, Tone, bool), HICON>,
}

impl IconCache {
    pub fn new(size: usize) -> Self {
        IconCache { size: size.clamp(16, 64), icons: HashMap::new() }
    }

    pub fn get(&mut self, percent_left: Option<f64>, light_taskbar: bool) -> HICON {
        let key = (level(percent_left), Tone::for_percent_left(percent_left), light_taskbar);
        let size = self.size;
        *self.icons.entry(key).or_insert_with(|| create_hicon(size, &render(size, key.0, key.1, key.2)))
    }
}

impl Drop for IconCache {
    fn drop(&mut self) {
        for (_, icon) in self.icons.drain() {
            if !icon.is_null() {
                unsafe { DestroyIcon(icon) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(px: &[u8], size: usize, x: usize, y: usize) -> u8 {
        px[(y * size + x) * 4 + 3]
    }

    fn rgb(px: &[u8], size: usize, x: usize, y: usize) -> [u8; 3] {
        let i = (y * size + x) * 4;
        [px[i + 2], px[i + 1], px[i]]
    }

    #[test]
    fn levels() {
        assert_eq!(level(None), 0);
        assert_eq!(level(Some(0.0)), 0);
        assert_eq!(level(Some(3.0)), 1);
        assert_eq!(level(Some(44.0)), 4);
        assert_eq!(level(Some(100.0)), 10);
        assert_eq!(Tone::for_percent_left(Some(30.0)), Tone::Low);
        assert_eq!(Tone::for_percent_left(Some(9.0)), Tone::Critical);
        assert_eq!(Tone::for_percent_left(Some(31.0)), Tone::Ok);
    }

    #[test]
    fn ring_geometry() {
        let s = 32;
        let px = render(s, 5, Tone::Ok, false);
        assert_eq!(px.len(), s * s * 4);
        // Centre is transparent (it's a ring).
        assert_eq!(alpha(&px, s, 16, 16), 0);
        // Right side (3 o'clock, inside the first half) is the coloured arc.
        assert_eq!(rgb(&px, s, 29, 16), Tone::Ok.rgb());
        assert!(alpha(&px, s, 29, 16) > 200);
        // Left side (9 o'clock, second half) is the faint track.
        let a = alpha(&px, s, 2, 16);
        assert!(a > 0 && a < 160, "track alpha {a}");
        // Corners are transparent.
        assert_eq!(alpha(&px, s, 0, 0), 0);
    }

    #[test]
    fn full_and_empty() {
        let s = 16;
        let full = render(s, 10, Tone::Ok, true);
        let empty = render(s, 0, Tone::NoData, true);
        assert!(alpha(&full, s, 1, 8) > 200);
        assert!(alpha(&empty, s, 1, 8) < 160);
        // Exhausted critical icon gets a centre dot.
        let dead = render(s, 0, Tone::Critical, false);
        assert!(alpha(&dead, s, 8, 8) > 200);
    }
}
