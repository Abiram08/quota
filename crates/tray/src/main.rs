//! quota-tray: a Windows notification-area icon over quota-core.
//!
//! Design for low resource use:
//! * One hidden window + one `SetTimer`; the thread sleeps in `GetMessageW`.
//! * Fetches run on a short-lived worker thread and are posted back.
//! * Icons are drawn once per level and cached; the popup exists only
//!   while open; the working set is trimmed after each refresh.

#![cfg_attr(not(test), windows_subsystem = "windows")]
// Both thread_locals already use `const { .. }`; clippy 1.95 still reports
// them (false positive), so silence just that lint.
#![allow(clippy::missing_const_for_thread_local)]

mod arc;
mod bench;
mod icon;
mod model;
mod preview;

/// Rust bindings generated from `ui/app.slint` by build.rs.
mod ui {
    slint::include_modules!();
}
mod popup;
mod sys;
#[cfg(test)]
mod theme_tests;

use quota_core::backoff::Backoff;
use quota_core::config::{Config, ResetTimers, Theme, UsageMode};
use quota_core::notify::{Alert, Tracker};
use quota_core::refresh::{self, FetchResult};
use quota_core::snapshot::Snapshot;
use quota_core::{format, output, paths, providers, time};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use sys::{copy_wide, wide};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::HICON;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize};
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForSystem, GetSystemMetricsForDpi, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_RESPECT_QUIET_TIME, NIIF_WARNING,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NINF_KEY, NIN_SELECT, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, KillTimer, PostMessageW, RegisterClassW, RegisterWindowMessageW,
    SetForegroundWindow, SetTimer, TrackPopupMenu, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    SM_CXSMICON, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_COMMAND, WM_CONTEXTMENU,
    WM_DESTROY, WM_NULL, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

const WM_TRAY: u32 = WM_APP + 1;
const WM_RESULTS: u32 = WM_APP + 2;
const TIMER_ID: usize = 1;
const ICON_UID: u32 = 1;
/// Keyboard activation of the icon (`NIN_SELECT | NINF_KEY`); not exported by windows-sys.
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const CMD_REFRESH: usize = 1;
const CMD_AUTOSTART: usize = 2;
const CMD_CONFIG: usize = 3;
const CMD_DATA: usize = 4;
const CMD_QUIT: usize = 9;
const CMD_THEME_SYSTEM: usize = 20;
const CMD_THEME_LIGHT: usize = 21;
const CMD_THEME_DARK: usize = 22;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

struct App {
    hwnd: HWND,
    cfg: Config,
    snapshot: Snapshot,
    backoff: HashMap<String, Backoff>,
    in_flight: bool,
    tracker: Tracker,
    icons: icon::IconCache,
    light_taskbar: bool,
    timer_secs: u64,
    /// Unix time of the next poll timer tick.
    next_tick: i64,
    /// Last icon and tip actually sent to the shell. The timer skips the
    /// notify call when neither changed, so an idle poll does no extra work.
    shown_icon: HICON,
    shown_tip: String,
    /// Token usage from local session logs; refreshed on the worker thread
    /// with every poll (incremental, ~70 ms after the first scan).
    tokens: Option<quota_core::tokens::Report>,
}

/// What the refresh worker hands back to the UI thread.
type WorkerResult = (Vec<FetchResult>, quota_core::tokens::Report);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app state. Returns `None` if the state is unavailable
/// (not yet created, or already borrowed by an outer handler).
pub(crate) fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

pub(crate) fn with_snapshot<R>(f: impl FnOnce(&Snapshot) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow().ok().and_then(|g| g.as_ref().map(|app| f(&app.snapshot))))
}

/// True while a background fetch is running (popup shows "Refreshing…").
pub(crate) fn is_refreshing() -> bool {
    APP.with(|a| a.try_borrow().ok().and_then(|g| g.as_ref().map(|app| app.in_flight))).unwrap_or(false)
}

/// The configured popup theme.
pub(crate) fn theme() -> Theme {
    APP.with(|a| a.try_borrow().ok().and_then(|g| g.as_ref().map(|app| app.cfg.theme))).unwrap_or_default()
}

/// Sets and saves the popup theme, then repaints an open popup.
pub(crate) fn set_theme(t: Theme) {
    with_app(|a| a.cfg.theme = t);
    let _ = Config::save_theme(t);
    popup::refresh();
}

/// A copy of the current config (defaults if the app state is busy).
pub(crate) fn config() -> Config {
    APP.with(|a| a.try_borrow().ok().and_then(|g| g.as_ref().map(|app| app.cfg.clone()))).unwrap_or_default()
}

/// Notifications on or off (Settings). Saved in place in config.toml.
pub(crate) fn set_notifications(on: bool) {
    if Config::save_value(Some("notifications"), "enabled", if on { "true" } else { "false" }).is_ok() {
        with_app(|a| a.cfg.notifications.enabled = on);
    }
    popup::refresh();
}

/// Bars show what is left, or what is used (Settings).
pub(crate) fn set_usage_mode(mode: UsageMode) {
    if Config::save_value(None, "usage_mode", &format!("\"{}\"", mode.as_str())).is_ok() {
        with_app(|a| a.cfg.usage_mode = mode);
    }
    popup::refresh();
}

/// Reset lines count down, or show a clock time (Settings).
pub(crate) fn set_reset_timers(mode: ResetTimers) {
    if Config::save_value(None, "reset_timers", &format!("\"{}\"", mode.as_str())).is_ok() {
        with_app(|a| a.cfg.reset_timers = mode);
    }
    popup::refresh();
}

/// Poll interval (Settings). Re-arms the timer right away.
pub(crate) fn set_poll_secs(secs: u64) {
    let secs = secs.max(quota_core::config::MIN_POLL_SECS);
    if Config::save_value(None, "poll_interval_secs", &secs.to_string()).is_ok() {
        with_app(|a| {
            a.cfg.poll_interval_secs = secs;
            a.ensure_timer();
        });
    }
    popup::refresh();
}

/// Shows or hides one provider (Settings). A provider turned on is
/// fetched at once; one turned off disappears from every tab.
pub(crate) fn toggle_provider(id: &str) {
    let list = model::toggled(&config().enabled_providers, id);
    let value = quota_core::config::toml_string_list(&list);
    if Config::save_value(None, "enabled_providers", &value).is_err() {
        return;
    }
    with_app(|a| {
        a.cfg.enabled_providers = list;
        a.snapshot.retain_order(&a.cfg.enabled_providers);
        a.update_tray(false);
        a.start_refresh(false);
    });
    popup::refresh();
}

/// Opens config.toml in Notepad (created with comments if missing).
pub(crate) fn open_config() {
    if let Ok(path) = Config::write_default_if_missing() {
        sys::shell_open(Some("notepad.exe"), &path);
    }
}

/// Starred metrics (`provider/Label`) from config.
/// Runs `f` on the latest token report (None until the first scan ends).
pub(crate) fn with_tokens<R>(f: impl FnOnce(Option<&quota_core::tokens::Report>) -> R) -> R {
    APP.with(|a| match a.try_borrow() {
        Ok(g) => f(g.as_ref().and_then(|app| app.tokens.as_ref())),
        Err(_) => f(None),
    })
}

/// Poll interval in seconds (config).
pub(crate) fn poll_secs() -> i64 {
    APP.with(|a| a.try_borrow().ok().and_then(|g| g.as_ref().map(|app| app.cfg.poll_interval_secs as i64))).unwrap_or(180)
}

pub(crate) fn pinned() -> Vec<String> {
    APP.with(|a| a.try_borrow().ok().and_then(|g| g.as_ref().map(|app| app.cfg.pinned.clone()))).unwrap_or_default()
}

/// Closes the app (tray menu or the popup's "..." menu).
pub(crate) fn quit() {
    let hwnd = APP.with(|a| a.try_borrow().ok().and_then(|g| g.as_ref().map(|app| app.hwnd)));
    if let Some(h) = hwnd {
        unsafe { DestroyWindow(h) };
    }
}

/// System -> Light -> Dark -> System.
pub(crate) fn cycle_theme() {
    set_theme(theme().next());
}

/// When the next fetch will actually happen: the first timer tick at or
/// after the earliest provider is due (providers in backoff wait longer).
pub(crate) fn next_update_at() -> Option<i64> {
    APP.with(|a| {
        let g = a.try_borrow().ok()?;
        let app = g.as_ref()?;
        let earliest = app
            .cfg
            .enabled_providers
            .iter()
            .map(|id| app.backoff.get(id).map(|b| b.next_due()).unwrap_or(0))
            .min()?;
        Some(next_update(app.next_tick, app.timer_secs as i64, earliest))
    })
}

/// Pure part of [`next_update_at`].
fn next_update(next_tick: i64, interval: i64, earliest_due: i64) -> i64 {
    if interval <= 0 || earliest_due <= next_tick {
        return next_tick;
    }
    let ticks = (earliest_due - next_tick + interval - 1) / interval;
    next_tick + ticks * interval
}

/// Fetches every enabled provider now, ignoring backoff.
pub(crate) fn refresh_now() {
    with_app(|a| a.start_refresh(true));
    popup::refresh();
}

/// Releases pages not in active use (the process stays fully functional;
/// pages are faulted back in on demand).
pub(crate) fn trim_working_set() {
    unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
}

fn icon_size() -> usize {
    let s = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
    if s > 0 {
        s as usize
    } else {
        16
    }
}

/// Tooltip: the three most urgent limits, compact (max 127 UTF-16 units).
fn tooltip(snap: &Snapshot, now: i64) -> String {
    let top = output::most_urgent(snap, now, 3);
    let mut lines: Vec<String> = top
        .iter()
        .map(|(name, l)| {
            let mut s = format!("{name} {}: {}% left", l.label, l.percent_left().floor() as i64);
            if let Some(r) = l.resets_at {
                s.push_str(&format!(" · {}", format::duration(r - now)));
            }
            s
        })
        .collect();
    if lines.is_empty() {
        let down: Vec<&str> = snap.providers.iter().filter(|p| p.is_unavailable()).map(|p| p.name.as_str()).collect();
        lines.push(if snap.providers.is_empty() {
            "Quota · fetching usage…".to_string()
        } else {
            format!("Quota · no limits ({} unavailable)", down.len())
        });
    }
    if snap.providers.iter().any(|p| p.stale) {
        lines.push("* some data is stale".into());
    }
    let mut out = String::new();
    for l in lines {
        let candidate = if out.is_empty() { l.clone() } else { format!("{out}\n{l}") };
        if candidate.encode_utf16().count() > 127 {
            break;
        }
        out = candidate;
    }
    out
}

fn base_nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = ICON_UID;
    nid
}

impl App {
    fn most_urgent_left(&self, now: i64) -> Option<f64> {
        output::most_urgent(&self.snapshot, now, 1).first().map(|(_, l)| l.percent_left())
    }

    /// Adds (after start or Explorer restart) or updates the tray icon.
    fn update_tray(&mut self, add: bool) {
        let now = time::now();
        let icon = self.icons.get(self.most_urgent_left(now), self.light_taskbar);
        let tip = tooltip(&self.snapshot, now);
        if !add && icon == self.shown_icon && tip == self.shown_tip {
            return;
        }
        let mut nid = base_nid(self.hwnd);
        nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_SHOWTIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = icon;
        copy_wide(&mut nid.szTip, &tip);
        let placed = unsafe {
            if add {
                let ok = Shell_NotifyIconW(NIM_ADD, &nid) != 0;
                nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &nid);
                ok
            } else if Shell_NotifyIconW(NIM_MODIFY, &nid) == 0 {
                // Icon vanished (e.g. Explorer restarted before we saw the message).
                let ok = Shell_NotifyIconW(NIM_ADD, &nid) != 0;
                nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &nid);
                ok
            } else {
                true
            }
        };
        // Remember only a successful update, so a failed add is retried.
        if placed {
            self.shown_icon = icon;
            self.shown_tip = tip;
        }
    }

    fn notify(&self, alerts: &[Alert]) {
        if alerts.is_empty() {
            return;
        }
        let (title, body) = if alerts.len() == 1 {
            (alerts[0].title.clone(), alerts[0].body.clone())
        } else {
            (
                format!("{} limits need attention", alerts.len()),
                alerts.iter().map(|a| a.title.as_str()).collect::<Vec<_>>().join("\n"),
            )
        };
        let mut nid = base_nid(self.hwnd);
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_WARNING | NIIF_RESPECT_QUIET_TIME;
        copy_wide(&mut nid.szInfoTitle, &title);
        copy_wide(&mut nid.szInfo, &body);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    pub(crate) fn ensure_timer(&mut self) {
        if self.timer_secs != self.cfg.poll_interval_secs {
            self.timer_secs = self.cfg.poll_interval_secs;
            self.next_tick = time::now() + self.timer_secs as i64;
            unsafe { SetTimer(self.hwnd, TIMER_ID, (self.timer_secs * 1000).min(u32::MAX as u64) as u32, None) };
        }
    }

    /// Starts a background fetch of every due provider (all if `force`).
    fn start_refresh(&mut self, force: bool) {
        self.cfg = Config::load().0;
        self.ensure_timer();
        if self.in_flight {
            return;
        }
        let now = time::now();
        if force {
            self.backoff.values_mut().for_each(Backoff::reset_due);
        }
        let due: Vec<String> = self
            .cfg
            .enabled_providers
            .iter()
            .filter(|id| self.backoff.get(*id).is_none_or(|b| b.is_due(now + 5)))
            .cloned()
            .collect();
        if due.is_empty() {
            return;
        }
        let cfg = self.cfg.clone();
        let hwnd = self.hwnd as isize;
        let spawned = std::thread::Builder::new()
            .name("quota-fetch".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let list: Vec<_> = due.iter().filter_map(|id| providers::by_id(id, &cfg)).collect();
                let refs: Vec<&dyn quota_core::Provider> = list.iter().map(|b| b.as_ref()).collect();
                let fetched = refresh::fetch_all(&refs);
                let tokens = quota_core::tokens::scan(time::now());
                let results: Box<WorkerResult> = Box::new((fetched, tokens));
                let ptr = Box::into_raw(results);
                // SAFETY: ownership passes to the UI thread via the message;
                // reclaimed here if posting fails.
                unsafe {
                    if PostMessageW(hwnd as HWND, WM_RESULTS, 0, ptr as LPARAM) == 0 {
                        drop(Box::from_raw(ptr));
                    }
                }
            });
        self.in_flight = spawned.is_ok();
    }

    fn on_results(&mut self, (results, tokens): WorkerResult) {
        self.tokens = Some(tokens);
        let now = time::now();
        let base = self.cfg.poll_interval_secs;
        for (id, _, r) in &results {
            self.backoff.entry(id.to_string()).or_default().record(r.is_ok(), base, now);
        }
        refresh::apply(&mut self.snapshot, results, now);
        self.snapshot.retain_order(&self.cfg.enabled_providers);
        let _ = self.snapshot.save();
        let alerts = self.tracker.evaluate(&self.snapshot, &self.cfg.notifications, now);
        if let Some(p) = paths::notified_file() {
            let _ = self.tracker.save(&p);
        }
        self.in_flight = false;
        self.update_tray(false);
        self.notify(&alerts);
    }
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let add = |flags: u32, id: usize, text: &str| {
            let w = wide(text);
            AppendMenuW(menu, flags, id, w.as_ptr());
        };
        add(MF_STRING | MF_GRAYED, 0, concat!("Quota ", env!("CARGO_PKG_VERSION")));
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        add(MF_STRING, CMD_REFRESH, "Refresh now");
        let auto = if sys::autostart_enabled() { MF_CHECKED } else { 0 };
        add(MF_STRING | auto, CMD_AUTOSTART, "Start with Windows");
        let sub = CreatePopupMenu();
        if !sub.is_null() {
            let current = theme();
            for (id, t) in [(CMD_THEME_SYSTEM, Theme::System), (CMD_THEME_LIGHT, Theme::Light), (CMD_THEME_DARK, Theme::Dark)] {
                let w = wide(t.label());
                AppendMenuW(sub, MF_STRING | if t == current { MF_CHECKED } else { 0 }, id, w.as_ptr());
            }
            let w = wide("Theme");
            AppendMenuW(menu, MF_STRING | MF_POPUP, sub as usize, w.as_ptr());
        }
        add(MF_STRING, CMD_CONFIG, "Open config file");
        add(MF_STRING, CMD_DATA, "Open data folder");
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        add(MF_STRING, CMD_QUIT, "Quit");

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Required so the menu closes when the user clicks elsewhere.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, std::ptr::null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        if cmd > 0 {
            on_command(hwnd, cmd as usize);
        }
    }
}

fn on_command(hwnd: HWND, cmd: usize) {
    match cmd {
        CMD_REFRESH => refresh_now(),
        CMD_AUTOSTART => {
            sys::set_autostart(!sys::autostart_enabled());
        }
        CMD_CONFIG => open_config(),
        CMD_DATA => {
            if let Some(dir) = paths::data_dir() {
                let _ = std::fs::create_dir_all(&dir);
                sys::shell_open(None, &dir);
            }
        }
        CMD_THEME_SYSTEM => set_theme(Theme::System),
        CMD_THEME_LIGHT => set_theme(Theme::Light),
        CMD_THEME_DARK => set_theme(Theme::Dark),
        CMD_QUIT => unsafe {
            DestroyWindow(hwnd);
        },
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let taskbar_created = TASKBAR_CREATED.load(Ordering::Relaxed);
    if taskbar_created != 0 && msg == taskbar_created {
        with_app(|a| a.update_tray(true));
        return 0;
    }
    match msg {
        WM_TRAY => {
            // NOTIFYICON_VERSION_4: event in LOWORD(lParam).
            match (lp as u32) & 0xFFFF {
                NIN_SELECT | NIN_KEYSELECT => popup::toggle(),
                WM_CONTEXTMENU => {
                    popup::close();
                    show_menu(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_RESULTS => {
            // SAFETY: produced by Box::into_raw in the worker thread.
            let results = *Box::from_raw(lp as *mut WorkerResult);
            with_app(|a| a.on_results(results));
            popup::refresh();
            trim_working_set();
            0
        }
        WM_TIMER if wp == TIMER_ID => {
            with_app(|a| {
                a.next_tick = time::now() + a.timer_secs as i64;
                a.start_refresh(false);
                // Keep the relative times in the tooltip current.
                a.update_tray(false);
            });
            popup::refresh();
            0
        }
        WM_SETTINGCHANGE => {
            with_app(|a| {
                let light = sys::taskbar_is_light();
                if light != a.light_taskbar {
                    a.light_taskbar = light;
                    a.update_tray(false);
                }
            });
            popup::refresh();
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_COMMAND => {
            on_command(hwnd, wp & 0xFFFF);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_ID);
            let nid = base_nid(hwnd);
            Shell_NotifyIconW(NIM_DELETE, &nid);
            popup::close();
            let _ = slint::quit_event_loop();
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// `quota-tray --preview <out.bmp> [--light|--dark] [--tab NAME] [--scale S]`
/// renders the popup off-screen (software renderer) and exits.
fn preview(args: &[String]) -> bool {
    let Some(i) = args.iter().position(|a| a == "--preview") else { return false };
    let out = args.get(i + 1).map(std::path::PathBuf::from).unwrap_or_else(|| "quota-preview.bmp".into());
    let (cfg, _) = Config::load();
    let theme = if args.iter().any(|a| a == "--light") {
        Theme::Light
    } else if args.iter().any(|a| a == "--dark") {
        Theme::Dark
    } else {
        cfg.theme
    };
    let scale = args
        .iter()
        .position(|a| a == "--scale")
        .and_then(|j| args.get(j + 1))
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(1.25)
        .clamp(1.0, 4.0);
    let mut snap = Snapshot::load_or_default();
    snap.retain_order(&cfg.enabled_providers);
    let tab = args.iter().position(|a| a == "--tab").and_then(|i| args.get(i + 1)).map(String::as_str);
    if let Err(e) = popup::render_preview(&snap, &out, theme, tab, scale, &cfg.pinned) {
        let _ = std::fs::write(out.with_extension("err.txt"), e);
    }
    true
}

/// Slint: winit backend, software renderer. Every popup window is created
/// without a taskbar button and with Windows 11 rounded corners.
fn select_backend() -> bool {
    use slint::winit_030::winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(|attrs| {
            attrs.with_skip_taskbar(true).with_corner_preference(CornerPreference::Round)
        })
        .select()
        .is_ok()
}

/// The hidden tray window (clipboard owner, menus).
pub(crate) fn main_hwnd() -> HWND {
    APP.with(|a| a.try_borrow().ok().and_then(|g| g.as_ref().map(|app| app.hwnd))).unwrap_or(std::ptr::null_mut())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if preview(&args) {
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--slint-probe") {
        let _ = select_backend();
        let out = args.get(i + 1).cloned().unwrap_or_else(|| "slint-probe.txt".into());
        bench::slint_probe(std::path::Path::new(&out));
        return;
    }
    if !select_backend() {
        return;
    }
    // `--bench-popup <out.txt> [--tab NAME]`: measure the popup, no tray icon,
    // no network, and no single-instance lock (runs beside a real tray).
    let bench_out = args.iter().position(|a| a == "--bench-popup").map(|i| {
        args.get(i + 1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "quota-bench.txt".into())
    });
    let bench_tab = args.iter().position(|a| a == "--tab").and_then(|i| args.get(i + 1)).cloned();
    unsafe {
        if bench_out.is_none() {
            // One instance per user session.
            let name = wide("Local\\quota-tray-single-instance");
            let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
                return;
            }
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide("quota_tray");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);

        // A hidden top-level window (not HWND_MESSAGE) so it receives the
        // TaskbarCreated broadcast after Explorer restarts.
        let title = wide("Quota");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }

        let (cfg, _) = Config::load();
        let mut snapshot = Snapshot::load_or_default();
        snapshot.retain_order(&cfg.enabled_providers);
        let tracker = paths::notified_file().map(|p| Tracker::load(&p)).unwrap_or_default();
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                hwnd,
                cfg,
                snapshot,
                backoff: HashMap::new(),
                in_flight: false,
                tracker,
                icons: icon::IconCache::new(icon_size()),
                light_taskbar: sys::taskbar_is_light(),
                timer_secs: 0,
                next_tick: 0,
                shown_icon: std::ptr::null_mut(),
                shown_tip: String::new(),
                tokens: None,
            })
        });
        if let Some(out) = bench_out {
            bench::run(std::path::Path::new(&out), bench_tab.as_deref());
            APP.with(|a| a.borrow_mut().take());
            return;
        }
        with_app(|a| {
            a.update_tray(true);
            a.start_refresh(true); // also arms the timer
        });

        // Slint's (winit) loop pumps every message of this thread, so the
        // hidden tray window, its timer and the worker's results all keep
        // arriving. It sleeps until something happens.
        let _ = slint::run_event_loop_until_quit();
        APP.with(|a| a.borrow_mut().take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quota_core::{Limit, ProviderError, Unit};

    fn lim(label: &str, used: f64) -> Limit {
        Limit {
            provider: "claude".into(),
            account_label: None,
            label: label.into(),
            used,
            limit: 100.0,
            unit: Unit::Percent,
            window_start: None,
            resets_at: Some(10_000),
        }
    }

    #[test]
    fn tooltip_top_three_and_length() {
        let mut s = Snapshot::default();
        s.apply(
            "claude",
            "Claude Code",
            Ok(vec![lim("Session (5h)", 18.0), lim("Weekly", 70.0), lim("Weekly · Some Long Model Name", 5.0), lim("Other", 1.0)]),
            0,
        );
        let t = tooltip(&s, 0);
        let lines: Vec<&str> = t.lines().collect();
        assert!(lines[0].starts_with("Claude Weekly: 30% left"), "{t}");
        assert!(lines.len() <= 3);
        assert!(t.encode_utf16().count() <= 127);
    }

    #[test]
    fn tooltip_without_data() {
        assert_eq!(tooltip(&Snapshot::default(), 0), "Quota · fetching usage…");
        let mut s = Snapshot::default();
        s.apply("cursor", "Cursor", Err(ProviderError::Unavailable("x".into())), 0);
        assert_eq!(tooltip(&s, 0), "Quota · no limits (1 unavailable)");
    }
}
