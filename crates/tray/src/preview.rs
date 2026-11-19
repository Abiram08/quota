//! Off-screen rendering with Slint's software renderer, for screenshots,
//! design review and tests. Installs a minimal platform (no winit window),
//! so it must run before anything else touches Slint, in its own process.

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::Rgb8Pixel;
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::PhysicalSize;
use std::rc::Rc;

struct Offscreen(Rc<MinimalSoftwareWindow>);

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// Installs the off-screen platform; returns the window to size and draw.
pub fn install(scale: f32) -> Option<Rc<MinimalSoftwareWindow>> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).ok()?;
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
    Some(window)
}

/// Renders the current contents of `window` at its size into a BMP file.
pub fn capture(window: &MinimalSoftwareWindow, size: PhysicalSize, path: &std::path::Path) -> std::io::Result<()> {
    window.set_size(size);
    slint::platform::update_timers_and_animations();
    let (w, h) = (size.width as usize, size.height as usize);
    let mut px = vec![Rgb8Pixel::default(); w * h];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut px, w);
    });
    std::fs::write(path, bmp24(w, h, &px))
}

/// Bottom-up 24-bit BMP.
fn bmp24(w: usize, h: usize, px: &[Rgb8Pixel]) -> Vec<u8> {
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for y in (0..h).rev() {
        let start = out.len();
        for p in &px[y * w..(y + 1) * w] {
            out.extend_from_slice(&[p.b, p.g, p.r]);
        }
        out.resize(start + row, 0);
    }
    out
}
