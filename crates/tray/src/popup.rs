//! The Slint popup: created when the tray icon is clicked, dropped (window
//! destroyed) when it closes, so nothing UI-related exists while closed.
//! Rust computes every value (`model.rs`, `arc.rs`); Slint only displays.

use crate::model::{self, Logo};
use crate::ui;
use quota_core::config::{ResetTimers, Theme, UsageMode};
use quota_core::tokens::Report;
use slint::{ComponentHandle, LogicalSize, ModelRc, PhysicalPosition, PhysicalSize, VecModel};
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// GitHub Sponsors page (also in README and .github/FUNDING.yml).
const SPONSOR_URL: &str = "https://github.com/sponsors/Abiram08";

thread_local! {
    static POPUP: RefCell<Option<ui::Popup>> = const { RefCell::new(None) };
    static LAST_CLOSED: Cell<Option<Instant>> = const { Cell::new(None) };
    static ANCHOR: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    static COPIED: Cell<bool> = const { Cell::new(false) };
    /// 0 = Priority, 1 = All usage. `--tab` sets it before the window opens.
    static START_VIEW: Cell<i32> = const { Cell::new(0) };
    /// Benchmark: keep the popup open when it loses focus.
    static BENCH: Cell<bool> = const { Cell::new(false) };
    static FRAMES: Cell<u32> = const { Cell::new(0) };
}

pub fn set_bench(on: bool) {
    BENCH.with(|b| b.set(on));
}

/// `--tab priority` opens Priority. `--tab usage` (and `all`) opens All usage.
pub fn set_start_tab(name: &str) {
    let view = match name {
        "usage" | "all" | "home" => 1,
        _ => 0,
    };
    START_VIEW.with(|v| v.set(view));
}

fn start_view() -> i32 {
    START_VIEW.with(|v| v.get())
}

/// Frames rendered so far (benchmark: "first frame done").
pub fn frame_count() -> u32 {
    FRAMES.with(|f| f.get())
}

pub fn is_open() -> bool {
    POPUP.with(|p| p.borrow().is_some())
}

/// Opens the popup, or closes it if open. A click that itself made the
/// popup lose focus (and close) does not immediately reopen it.
pub fn toggle() {
    if is_open() {
        close();
        return;
    }
    let just_closed = LAST_CLOSED.with(|c| c.get()).is_some_and(|t| t.elapsed() < Duration::from_millis(400));
    if !just_closed {
        open_now();
    }
}

pub fn close() {
    // Take it out first so nothing re-enters a borrowed popup.
    let ui = POPUP.with(|p| p.borrow_mut().take());
    if let Some(ui) = ui {
        let _ = ui.hide();
        drop(ui); // destroys the window
        LAST_CLOSED.with(|c| c.set(Some(Instant::now())));
        crate::trim_working_set();
    }
}

fn to_ui_logo(l: Logo) -> ui::Logo {
    match l {
        Logo::None => ui::Logo::None,
        Logo::Claude => ui::Logo::Claude,
        Logo::OpenAi => ui::Logo::Openai,
        Logo::Cursor => ui::Logo::Cursor,
        Logo::Copilot => ui::Logo::Copilot,
        Logo::OpenRouter => ui::Logo::Openrouter,
    }
}

fn model_rc<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn to_ui_cards(cards: &[model::UsageCard]) -> ModelRc<ui::UsageCardData> {
    model_rc(
        cards
            .iter()
            .map(|c| ui::UsageCardData {
                logo: to_ui_logo(c.logo),
                initial: c.initial.as_str().into(),
                name: c.name.as_str().into(),
                stale: c.stale,
                error: c.error.as_str().into(),
                hint: c.hint.as_str().into(),
                connected: c.connected,
                lines: model_rc(
                    c.lines
                        .iter()
                        .map(|l| ui::UsageLineData {
                            header: l.header.as_str().into(),
                            label: l.label.as_str().into(),
                            kind: l.kind,
                            fill: l.fill as f32,
                            amount: l.amount.as_str().into(),
                            reset: l.reset.as_str().into(),
                            tone: l.tone,
                            copy: l.copy.as_str().into(),
                            a11y: l.a11y.as_str().into(),
                        })
                        .collect(),
                ),
            })
            .collect(),
    )
}

fn to_ui_rows(rows: &[model::PriorityRow]) -> ModelRc<ui::PriorityRowData> {
    model_rc(
        rows
            .iter()
            .map(|r| ui::PriorityRowData {
                heading: r.heading.as_str().into(),
                logo: to_ui_logo(r.logo),
                initial: r.initial.as_str().into(),
                title: r.title.as_str().into(),
                subtitle: r.subtitle.as_str().into(),
                percent: r.percent.as_str().into(),
                reset: r.reset.as_str().into(),
                tone: r.tone,
                used_up: r.used_up,
                track_path: r.track_path.as_str().into(),
                arc_path: r.arc_path.as_str().into(),
                copy: r.copy.as_str().into(),
                a11y: r.a11y.as_str().into(),
            })
            .collect(),
    )
}

/// Light or dark after resolving "System".
/// Effective DPI of a monitor as a scale factor (1.25 at 125%).
fn monitor_scale(mon: windows_sys::Win32::Graphics::Gdi::HMONITOR) -> Option<f32> {
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    let (mut x, mut y) = (0u32, 0u32);
    let ok = unsafe { GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y) } == 0;
    (ok && x > 0).then(|| x as f32 / 96.0)
}

/// The popup's native window, once it exists.
fn hwnd(ui: &ui::Popup) -> Option<windows_sys::Win32::Foundation::HWND> {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    ui.window()
        .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get() as windows_sys::Win32::Foundation::HWND),
            _ => None,
        })
        .flatten()
}

/// Rounded corners, shadow and a border in the current theme's colour.
fn apply_chrome(ui: &ui::Popup) {
    if let Some(h) = hwnd(ui) {
        let c = ui.global::<ui::Theme>().get_control_edge();
        crate::sys::style_popup(h, [c.red(), c.green(), c.blue()]);
    }
}

pub fn is_dark(theme: Theme) -> bool {
    match theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => !crate::sys::apps_are_light(),
    }
}

/// Everything `fill` reads besides the snapshot.
pub struct FillArgs<'a> {
    pub now: i64,
    pub theme: Theme,
    pub pinned: &'a [String],
    pub footer: &'a str,
    /// Twice the poll interval: older data is shown as stale.
    pub stale_after: i64,
    /// Providers turned on in Settings, in order.
    pub enabled: &'a [String],
    pub notify: bool,
    pub poll_secs: u64,
    pub autostart: bool,
    pub usage_mode: UsageMode,
    pub reset_timers: ResetTimers,
    /// Notification thresholds: running-out, then red.
    pub low_left: f64,
    pub hot_left: f64,
}

/// Fills every property from the snapshot. `report` is the token scan
/// (None until the first scan finishes).
pub fn fill(ui: &ui::Popup, snap: &quota_core::snapshot::Snapshot, a: &FillArgs, report: Option<&Report>) {
    let t = ui.global::<ui::Theme>();
    t.set_dark(is_dark(a.theme));
    t.set_reduce_motion(crate::sys::reduce_motion());
    apply_chrome(ui);
    let q = model::HomeQuery {
        now: a.now,
        stale_after: a.stale_after,
        mode: a.usage_mode,
        timers: a.reset_timers,
        low_left: a.low_left,
        hot_left: a.hot_left,
        pinned: a.pinned,
        report,
    };
    let cards = model::usage_cards(snap, a.enabled, &q);
    ui.set_show_empty(model::show_empty(&cards));
    // Connected providers get a card each; the rest share one card.
    let (on, off): (Vec<model::UsageCard>, Vec<model::UsageCard>) = cards.iter().cloned().partition(|c| c.connected);
    ui.set_cards(to_ui_cards(&on));
    ui.set_offline(to_ui_cards(&off));
    ui.set_rows(to_ui_rows(&model::priority_rows(snap, a.enabled, &q)));
    ui.set_app_version(env!("CARGO_PKG_VERSION").into());
    let connectors = model::connectors(snap, Some(a.enabled));
    ui.set_welcome(model::is_welcome(snap));
    ui.set_connect_summary(
        format!("{} of {} connected", connectors.iter().filter(|c| c.connected).count(), connectors.len()).into(),
    );
    ui.set_connectors(model_rc(
        connectors
            .iter()
            .map(|c| ui::ConnectorData {
                logo: to_ui_logo(c.logo),
                initial: c.initial.as_str().into(),
                name: c.name.as_str().into(),
                label: c.label.as_str().into(),
                state: c.state.as_str().into(),
                how: c.how.as_str().into(),
                connected: c.connected,
            })
            .collect(),
    ));
    ui.set_footer(a.footer.into());
    ui.set_theme_label(a.theme.label().into());
    // Settings page.
    ui.set_autostart(a.autostart);
    ui.set_notify(a.notify);
    ui.set_theme_index(model::theme_index(a.theme));
    ui.set_usage_index(match a.usage_mode {
        UsageMode::Left => 0,
        UsageMode::Used => 1,
    });
    ui.set_reset_index(match a.reset_timers {
        ResetTimers::Relative => 0,
        ResetTimers::Absolute => 1,
    });
    let (poll_index, poll_note) = model::poll_choice(a.poll_secs);
    ui.set_poll_index(poll_index);
    ui.set_poll_note(poll_note.into());
    ui.set_provider_toggles(model_rc(
        model::provider_toggles(snap, a.enabled)
            .iter()
            .map(|p| ui::ProviderToggleData {
                logo: to_ui_logo(p.logo),
                initial: p.initial.as_str().into(),
                name: p.name.as_str().into(),
                state: p.state.as_str().into(),
                enabled: p.enabled,
            })
            .collect(),
    ));
}

fn fill_live(ui: &ui::Popup) {
    let now = quota_core::time::now();
    let mut footer = model::footer(now, crate::is_refreshing(), crate::next_update_at());
    if COPIED.with(|c| c.get()) {
        footer = format!("Copied · {footer}");
    }
    let pinned = crate::pinned();
    let cfg = crate::config();
    let (low_left, hot_left) = model::level_bands(&cfg.notifications.thresholds_percent_left);
    let args = FillArgs {
        now,
        theme: crate::theme(),
        pinned: &pinned,
        footer: &footer,
        stale_after: 2 * crate::poll_secs(),
        enabled: &cfg.enabled_providers,
        notify: cfg.notifications.enabled,
        poll_secs: cfg.poll_interval_secs,
        autostart: crate::sys::autostart_enabled(),
        usage_mode: cfg.usage_mode,
        reset_timers: cfg.reset_timers,
        low_left,
        hot_left,
    };
    let empty = quota_core::snapshot::Snapshot::default();
    // Shared borrows: the token report and the snapshot can be read together.
    crate::with_tokens(|report| {
        if crate::with_snapshot(|s| fill(ui, s, &args, report)).is_none() {
            fill(ui, &empty, &args, report);
        }
    });
}

/// Sizes the window to its content and places it at the taskbar corner of
/// the monitor the click came from (any taskbar edge, any DPI).
fn place(ui: &ui::Popup) {
    let (ax, ay) = ANCHOR.with(|a| a.get());
    unsafe {
        let mon = MonitorFromPoint(POINT { x: ax, y: ay }, MONITOR_DEFAULTTONEAREST);
        // The target monitor's own scale. Before the window exists Slint
        // reports 1.0, which put the popup partly off-screen at 125%+.
        let scale = monitor_scale(mon).unwrap_or_else(|| ui.window().scale_factor());
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let work = mi.rcWork;
        let margin = (12.0 * scale) as i32;
        // Cap the body so the popup always fits the work area.
        let avail_logical = ((work.bottom - work.top - 2 * margin) as f32 / scale).max(200.0);
        // The view switch and the footer sit outside the scrolling body.
        ui.set_window_limit(avail_logical);
        ui.set_max_body_height((avail_logical - 120.0).max(160.0));
        let width = ui.global::<ui::Theme>().get_popup_width();
        let h_logical = ui.get_content_height().min(avail_logical);
        ui.window().set_size(LogicalSize::new(width, h_logical));
        let (w, h) = ((width * scale).round() as i32, (h_logical * scale).round() as i32);
        let x = (ax - w / 2).clamp(work.left + margin, (work.right - w - margin).max(work.left + margin));
        let below_middle = ay > (work.top + work.bottom) / 2;
        let y = if below_middle { work.bottom - h - margin } else { work.top + margin };
        ui.window().set_position(PhysicalPosition::new(x, y));
    }
}

pub fn open_now() {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    ANCHOR.with(|a| a.set((pt.x, pt.y)));
    COPIED.with(|c| c.set(false));
    let Ok(ui) = ui::Popup::new() else { return };
    fill_live(&ui);
    ui.set_home_view(start_view());
    // First run stays on the welcome (Add an Account), not the sheet.

    ui.on_connect_opened(refresh);
    ui.on_check_again(crate::refresh_now);
    ui.on_refresh(crate::refresh_now);
    ui.on_cycle_theme(crate::cycle_theme);
    ui.on_quit(|| {
        close();
        crate::quit();
    });
    ui.on_close_popup(|| {
        // Defer: never destroy the window from inside its own callback.
        slint::Timer::single_shot(Duration::ZERO, close);
    });
    ui.on_copy_summary(copy_summary);
    ui.on_copy_reset(copy_line);
    ui.on_settings_opened(refresh);
    ui.on_toggle_autostart(|| {
        crate::sys::set_autostart(!crate::sys::autostart_enabled());
        refresh();
    });
    ui.on_toggle_notify(|| crate::set_notifications(!crate::config().notifications.enabled));
    ui.on_theme_selected(|i| {
        use quota_core::config::Theme;
        crate::set_theme([Theme::System, Theme::Light, Theme::Dark][i.clamp(0, 2) as usize]);
    });
    ui.on_usage_selected(|i| crate::set_usage_mode(if i == 1 { UsageMode::Used } else { UsageMode::Left }));
    ui.on_reset_selected(|i| crate::set_reset_timers(if i == 1 { ResetTimers::Absolute } else { ResetTimers::Relative }));
    ui.on_poll_selected(|i| {
        if let Some(secs) = model::POLL_CHOICES.get(i.max(0) as usize) {
            crate::set_poll_secs(*secs);
        }
    });
    ui.on_toggle_provider(|i| {
        if let Some(id) = quota_core::providers::ALL_IDS.get(i.max(0) as usize) {
            crate::toggle_provider(id);
        }
    });
    ui.on_open_config(crate::open_config);
    ui.on_open_sponsor(|| {
        // Opens the browser; Quota itself makes no network request.
        crate::sys::shell_open(None, std::path::Path::new(SPONSOR_URL));
    });

    // Close on click-away (focus lost), except while benchmarking.
    {
        use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
        ui.window().on_winit_window_event(|_, ev| {
            match ev {
                winit::event::WindowEvent::Focused(false) if !BENCH.with(|b| b.get()) => {
                    slint::Timer::single_shot(Duration::ZERO, close);
                }
                // Slint draws while handling RedrawRequested, right after this
                // callback. A zero timer runs once that frame is done. (The
                // rendering notifier is not supported by the software renderer.)
                winit::event::WindowEvent::RedrawRequested => {
                    slint::Timer::single_shot(Duration::ZERO, || FRAMES.with(|f| f.set(f.get().wrapping_add(1))));
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }
    place(&ui);
    let _ = ui.show();
    place(&ui); // the window now exists: apply the final position
    apply_chrome(&ui);
    ui.set_shown(true); // starts the fade/slide-in
    POPUP.with(|p| *p.borrow_mut() = Some(ui));
}

/// Re-fills an open popup (new data, theme, countdown). No-op when closed.
pub fn refresh() {
    POPUP.with(|p| {
        if let Some(ui) = p.borrow().as_ref() {
            fill_live(ui);
            place(ui);
        }
    });
}

fn copy_line(text: slint::SharedString) {
    let text = text.to_string();
    if text.is_empty() || !crate::sys::copy_text(crate::main_hwnd(), &text) {
        return;
    }
    COPIED.with(|c| c.set(true));
    refresh();
    slint::Timer::single_shot(Duration::from_millis(1500), || {
        COPIED.with(|c| c.set(false));
        refresh();
    });
}

fn copy_summary() {
    let now = quota_core::time::now();
    let text = crate::with_snapshot(|s| quota_core::output::plain_summary(s, now)).unwrap_or_default();
    if text.is_empty() || !crate::sys::copy_text(crate::main_hwnd(), &text) {
        return;
    }
    COPIED.with(|c| c.set(true));
    refresh();
    slint::Timer::single_shot(Duration::from_millis(1500), || {
        COPIED.with(|c| c.set(false));
        refresh();
    });
}

/// `quota-tray --preview <out.bmp> [--light|--dark] [--tab NAME] [--scale S]`:
/// renders the popup off-screen with the software renderer.
pub fn render_preview(
    snap: &quota_core::snapshot::Snapshot,
    out: &std::path::Path,
    theme: Theme,
    tab: Option<&str>,
    scale: f32,
    pinned: &[String],
) -> Result<(), String> {
    let window = crate::preview::install(scale).ok_or("could not install the preview platform")?;
    let (cfg, _) = quota_core::config::Config::load();
    let ui = ui::Popup::new().map_err(|e| e.to_string())?;
    // Previews show the whole page (no screen to fit, so no scrolling).
    ui.set_window_limit(4000.0);
    ui.set_max_body_height(4000.0);
    if let Some(t) = tab {
        set_start_tab(t);
    }
    let welcome = std::env::args().any(|a| a == "--welcome");
    ui.set_home_view(start_view());
    ui.set_settings_open(std::env::args().any(|a| a == "--settings"));
    ui.set_menu_open(std::env::args().any(|a| a == "--menu"));
    ui.set_about_open(std::env::args().any(|a| a == "--about"));
    ui.set_connect_open(std::env::args().any(|a| a == "--connect"));
    // --welcome: the first-run state, with every provider not signed in.
    let first_run = {
        let mut s = quota_core::snapshot::Snapshot::default();
        for (id, name) in [("claude", "Claude Code"), ("codex", "Codex"), ("cursor", "Cursor"), ("copilot", "GitHub Copilot"), ("openrouter", "OpenRouter")] {
            s.apply(id, name, Err(quota_core::ProviderError::NotConfigured("not signed in".into())), 0);
        }
        s
    };
    let snap = if welcome { &first_run } else { snap };
    let now = quota_core::time::now();
    let footer = model::footer(now, false, Some(now + 300));
    let report = quota_core::tokens::scan(now);
    let (low_left, hot_left) = model::level_bands(&cfg.notifications.thresholds_percent_left);
    let args = FillArgs {
        now,
        theme,
        pinned,
        footer: &footer,
        stale_after: 360,
        enabled: &cfg.enabled_providers,
        notify: cfg.notifications.enabled,
        poll_secs: cfg.poll_interval_secs,
        autostart: crate::sys::autostart_enabled(),
        usage_mode: cfg.usage_mode,
        reset_timers: cfg.reset_timers,
        low_left,
        hot_left,
    };
    fill(&ui, snap, &args, Some(&report));
    ui.global::<ui::Theme>().set_reduce_motion(true);
    ui.set_still(true);
    ui.set_shown(true);
    ui.show().map_err(|e| e.to_string())?;
    let width = ui.global::<ui::Theme>().get_popup_width();
    let h = ui.get_content_height();
    let size = PhysicalSize::new((width * scale).round() as u32, (h * scale).round() as u32);
    crate::preview::capture(&window, size, out).map_err(|e| e.to_string())
}
