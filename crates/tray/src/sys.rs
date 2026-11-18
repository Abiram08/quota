//! Thin Win32 helpers: UTF-16 strings, theme detection, autostart registry
//! value, opening files.

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// NUL-terminated UTF-16.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copies `s` into a fixed UTF-16 buffer, truncating on a char boundary
/// and always leaving room for the terminating NUL.
pub fn copy_wide(dst: &mut [u16], s: &str) {
    let max = dst.len().saturating_sub(1);
    let mut n = 0;
    for ch in s.chars() {
        let mut buf = [0u16; 2];
        let enc = ch.encode_utf16(&mut buf);
        if n + enc.len() > max {
            break;
        }
        dst[n..n + enc.len()].copy_from_slice(enc);
        n += enc.len();
    }
    for d in &mut dst[n..] {
        *d = 0;
    }
}

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "quota";

fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
    let (k, v) = (wide(subkey), wide(value));
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        )
    };
    (rc == ERROR_SUCCESS).then_some(data)
}

fn reg_string(subkey: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(subkey), wide(value));
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut size,
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Taskbar/tray theme (`SystemUsesLightTheme`); default dark.
pub fn taskbar_is_light() -> bool {
    reg_dword(PERSONALIZE, "SystemUsesLightTheme") == Some(1)
}

/// App theme (`AppsUseLightTheme`) for the popup; default light.
pub fn apps_are_light() -> bool {
    reg_dword(PERSONALIZE, "AppsUseLightTheme") != Some(0)
}

/// True when Windows "Animation effects" is off (Settings > Accessibility >
/// Visual effects). All popup animation is disabled then.
pub fn reduce_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::SystemParametersInfoW;
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    let mut enabled: i32 = 1;
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut enabled as *mut i32 as *mut _, 0) };
    ok != 0 && enabled == 0
}

fn exe_command() -> Option<String> {
    std::env::current_exe().ok().map(|p| format!("\"{}\"", p.display()))
}

fn run_value_matches_exe(value: &str) -> bool {
    match (reg_string(RUN_KEY, value), exe_command()) {
        (Some(v), Some(me)) => v.eq_ignore_ascii_case(&me),
        _ => false,
    }
}

/// True if the Run value points at this executable.
pub fn autostart_enabled() -> bool {
    run_value_matches_exe(RUN_VALUE)
}

fn set_run_value(name: &str, on: bool) -> bool {
    let (k, v) = (wide(RUN_KEY), wide(name));
    unsafe {
        if on {
            let Some(cmd) = exe_command() else { return false };
            let data = wide(&cmd);
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                v.as_ptr(),
                REG_SZ,
                data.as_ptr() as *const _,
                (data.len() * 2) as u32,
            ) == ERROR_SUCCESS
        } else {
            let rc = RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr());
            rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND
        }
    }
}

pub fn set_autostart(on: bool) -> bool {
    set_run_value(RUN_VALUE, on)
}

/// Opens a file or folder with its default handler (or `program` if given).
pub fn shell_open(program: Option<&str>, target: &std::path::Path) {
    let target = wide(&target.display().to_string());
    let verb = wide("open");
    unsafe {
        match program {
            Some(p) => {
                let p = wide(p);
                ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), p.as_ptr(), target.as_ptr(), std::ptr::null(), SW_SHOWNORMAL);
            }
            None => {
                ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
            }
        }
    }
}

/// Puts plain text on the clipboard (CF_UNICODETEXT). Returns success.
pub fn copy_text(owner: windows_sys::Win32::Foundation::HWND, text: &str) -> bool {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    const CF_UNICODETEXT: u32 = 13;
    let w = wide(text);
    unsafe {
        if OpenClipboard(owner) == 0 {
            return false;
        }
        EmptyClipboard();
        let bytes = w.len() * 2;
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
        let mut ok = false;
        if !h.is_null() {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                GlobalUnlock(h);
                // On success the clipboard owns the memory.
                ok = !SetClipboardData(CF_UNICODETEXT, h as _).is_null();
            }
            if !ok {
                GlobalFree(h);
            }
        }
        CloseClipboard();
        ok
    }
}

/// Windows 11 window chrome for the frameless popup: rounded corners, the
/// system drop shadow and a 1px border in the theme's border colour, so it
/// looks like a web popover rather than a flat rectangle. On Windows 10 the
/// corner and border attributes do not exist; the calls fail and are ignored.
pub fn style_popup(hwnd: windows_sys::Win32::Foundation::HWND, border_rgb: [u8; 3]) {
    use std::ffi::c_void;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_ROUND,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;
    if hwnd.is_null() {
        return;
    }
    let corner = DWMWCP_ROUND;
    // COLORREF is 0x00BBGGRR.
    let border: u32 = border_rgb[0] as u32 | (border_rgb[1] as u32) << 8 | (border_rgb[2] as u32) << 16;
    // A one-pixel frame lets DWM draw its shadow around a frameless window.
    let margins = MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 };
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as _,
            &corner as *const _ as *const c_void,
            std::mem::size_of_val(&corner) as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as _,
            &border as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_wide_truncates_and_terminates() {
        let mut buf = [0xFFFFu16; 5];
        copy_wide(&mut buf, "abcdefg");
        assert_eq!(&buf, &[97, 98, 99, 100, 0]);
        let mut buf = [0xFFFFu16; 4];
        // '😀' needs two UTF-16 units; it must not be split.
        copy_wide(&mut buf, "ab😀");
        assert_eq!(&buf, &[97, 98, 0, 0]);
        assert_eq!(wide("a"), vec![97, 0]);
    }
}
