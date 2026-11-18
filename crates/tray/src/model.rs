//! Snapshot -> plain rows for the popup. Pure and unit-tested; the Slint
//! files only display what this produces.
//!
//! The home popup follows the OpenUsage menu (MIT; see THIRD_PARTY.md):
//! Priority (pinned, running out, used up) and All usage (one card per
//! provider). The menu shows what is left and when it resets. Pace stays
//! in the CLI, the tray tooltip and notifications.

use quota_core::config::{ResetTimers, UsageMode};
use quota_core::snapshot::Snapshot;
use quota_core::tokens::{Report, Source};
use quota_core::{format, output, Limit, Unit};

/// Mirrors `Logo` in ui/components/logo.slint. `None` = show the initial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logo {
    None,
    Claude,
    OpenAi,
    Cursor,
    Copilot,
    OpenRouter,
}

pub fn logo_for(provider: &str) -> Logo {
    match provider {
        "claude" => Logo::Claude,
        "codex" | "chatgpt" | "openai" => Logo::OpenAi,
        "cursor" => Logo::Cursor,
        "copilot" => Logo::Copilot,
        "openrouter" => Logo::OpenRouter,
        _ => Logo::None,
    }
}

fn initial(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default()
}

/// Short names on the home cards and the Add account page.
/// Codex is the tool; the menu calls that account ChatGPT.
pub fn popup_name(id: &str) -> &'static str {
    match id {
        "claude" => "Claude",
        "codex" => "ChatGPT",
        "cursor" => "Cursor",
        "copilot" => "Copilot",
        "openrouter" => "OpenRouter",
        _ => "Provider",
    }
}

/// `Next update in 5m` / `Updating…`.
pub fn footer(now: i64, refreshing: bool, next_update_at: Option<i64>) -> String {
    if refreshing {
        return "Updating…".into();
    }
    match next_update_at {
        Some(t) if t > now => format!("Next update in {}", format::duration(t - now)),
        Some(_) => "Updating soon".into(),
        None => "Paused".into(),
    }
}

/// First run: every provider has been checked and none is connected, so
/// the popup opens on the welcome page instead of an empty home page.
pub fn is_welcome(snap: &Snapshot) -> bool {
    !snap.providers.is_empty() && snap.providers.iter().all(|p| p.is_unavailable())
}


fn amount_text(v: f64, unit: Unit) -> String {
    match unit {
        Unit::Percent => format!("{}%", v.max(0.0).floor() as i64),
        Unit::Requests => format!("{} req", v.max(0.0).round() as i64),
        _ => format::amount(v.max(0.0), unit),
    }
}

fn token_source(id: &str) -> Option<Source> {
    match id {
        "claude" => Some(Source::Claude),
        "codex" => Some(Source::Codex),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Priority and All usage

/// What the home pages need besides the snapshot. `low_left` and `hot_left`
/// are the notification thresholds (30 and 10 unless the user changed them):
/// at or under `low_left` a limit is running out, and at or under `hot_left`
/// its bar is red.
#[derive(Clone, Copy)]
pub struct HomeQuery<'a> {
    pub now: i64,
    pub stale_after: i64,
    pub mode: UsageMode,
    pub timers: ResetTimers,
    pub low_left: f64,
    pub hot_left: f64,
    pub pinned: &'a [String],
    pub report: Option<&'a Report>,
}

/// `(running-out threshold, red threshold)` from the user's notice list.
/// The list is stored highest first. An empty list uses 30 and 10.
pub fn level_bands(thresholds: &[f64]) -> (f64, f64) {
    let low = thresholds.first().copied().unwrap_or(30.0);
    let hot = thresholds.last().copied().unwrap_or(10.0).min(low);
    (low, hot)
}

fn tone_of(left: f64, low: f64, hot: f64) -> i32 {
    if left <= hot {
        2
    } else if left <= low {
        1
    } else {
        0
    }
}

fn used_up(l: &Limit) -> bool {
    (l.limit > 0.0 && l.used >= l.limit) || l.percent_left() <= 0.0
}

/// Fill fraction and the number beside the bar. Percent limits show `64%`
/// with no "left" or "used"; other units show the amount (`$15.00`).
fn beside(l: &Limit, mode: UsageMode) -> (f64, String) {
    let left = l.percent_left();
    match (mode, l.unit) {
        (UsageMode::Left, Unit::Percent) => (left / 100.0, format!("{}%", left.floor() as i64)),
        (UsageMode::Used, Unit::Percent) => ((100.0 - left) / 100.0, format!("{}%", (100.0 - left).floor() as i64)),
        (UsageMode::Left, u) => (left / 100.0, amount_text((l.limit - l.used).max(0.0), u)),
        (UsageMode::Used, u) => ((100.0 - left) / 100.0, amount_text(l.used.max(0.0), u)),
    }
}

fn left_words(l: &Limit, mode: UsageMode) -> String {
    let left = l.percent_left();
    match (mode, l.unit) {
        (UsageMode::Left, Unit::Percent) => format!("{}% left", left.floor() as i64),
        (UsageMode::Used, Unit::Percent) => format!("{}% used", (100.0 - left).floor() as i64),
        (UsageMode::Left, u) => format!("{} left", amount_text((l.limit - l.used).max(0.0), u)),
        (UsageMode::Used, u) => format!("{} used", amount_text(l.used.max(0.0), u)),
    }
}

/// `Resets in 3h 38m` / `Resets Wed 21:44`; empty when there is no date.
fn reset_phrase(l: &Limit, now: i64, timers: ResetTimers) -> String {
    match l.resets_at {
        Some(t) if timers == ResetTimers::Absolute => format!("Resets {}", format::clock(t, now)),
        Some(t) => format!("Resets in {}", format::duration((t - now).max(0))),
        None => String::new(),
    }
}

fn copy_of(name: &str, label: &str, l: &Limit, now: i64, timers: ResetTimers) -> String {
    let when = reset_phrase(l, now, timers);
    let when = if when.is_empty() { "No reset date".to_string() } else { when };
    format!("{name} {label}: {when}")
}

/// One line on an All usage card.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageLine {
    pub header: String,
    pub label: String,
    /// 0 = bar, 1 = text (no bar).
    pub kind: i32,
    pub fill: f64,
    pub amount: String,
    pub reset: String,
    pub tone: i32,
    pub copy: String,
    pub a11y: String,
}

/// One provider on All usage.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageCard {
    pub id: String,
    pub logo: Logo,
    pub initial: String,
    pub name: String,
    pub stale: bool,
    pub error: String,
    pub hint: String,
    pub connected: bool,
    pub lines: Vec<UsageLine>,
}

fn usage_line(name: &str, l: &Limit, header: String, q: &HomeQuery) -> UsageLine {
    let copy = copy_of(name, &l.label, l, q.now, q.timers);
    if l.has_reset(q.now) {
        return UsageLine {
            header,
            label: l.label.clone(),
            kind: 1,
            fill: 0.0,
            amount: "—".into(),
            reset: "Window ended · refresh to update".into(),
            tone: 0,
            copy,
            a11y: format!("{}: the window ended, refresh to update", l.label),
        };
    }
    let left = l.percent_left();
    let (fill, amount) = beside(l, q.mode);
    let tone = tone_of(left, q.low_left, q.hot_left);
    let reset = reset_phrase(l, q.now, q.timers);
    let mut a11y = format!("{}: {amount}", l.label);
    if !reset.is_empty() {
        a11y.push_str(&format!(", {reset}"));
    }
    if used_up(l) {
        a11y.push_str(", used up");
    } else if tone == 2 {
        a11y.push_str(", almost gone");
    } else if tone == 1 {
        a11y.push_str(", running low");
    }
    UsageLine { header, label: l.label.clone(), kind: 0, fill: fill.clamp(0.0, 1.0), amount, reset, tone, copy, a11y }
}

fn today_line(id: &str, report: Option<&Report>, now: i64) -> Option<UsageLine> {
    let src = token_source(id)?;
    let r = report?;
    if r.get(src).files == 0 {
        return None;
    }
    let today = quota_core::tokens::local_day(now);
    let total = r.sum(src, today, today);
    // A scan that found nothing today is not a row. "0 tokens" is noise.
    if total == 0 {
        return None;
    }
    let amount = format!("{} tokens", format::count(total as f64));
    Some(UsageLine {
        header: String::new(),
        label: "Today".into(),
        kind: 1,
        fill: 0.0,
        amount: amount.clone(),
        reset: String::new(),
        tone: 0,
        copy: String::new(),
        a11y: format!("Today, {amount}"),
    })
}

/// One card per enabled provider, in settings order. Accounts with different
/// labels become separate groups inside the card. Quota does not invent a
/// second account: a tool that reports one plan is one group.
pub fn usage_cards(snap: &Snapshot, enabled: &[String], q: &HomeQuery) -> Vec<UsageCard> {
    let hints = connectors(snap, None);
    enabled
        .iter()
        .map(|id| {
            let name = popup_name(id).to_string();
            let hint = hints.iter().find(|c| c.id == id).map(|c| c.how.clone()).unwrap_or_default();
            let mut card = UsageCard {
                id: id.clone(),
                logo: logo_for(id),
                initial: initial(&name),
                name: name.clone(),
                stale: false,
                error: String::new(),
                hint: String::new(),
                connected: false,
                lines: vec![],
            };
            match snap.provider(id) {
                None => {
                    card.error = "Not checked yet".into();
                    card.hint = "Quota checks it on the next update.".into();
                }
                Some(p) if p.is_unavailable() => {
                    card.error = output::sentence_case(p.error.as_ref().map(|e| e.message.as_str()).unwrap_or("unavailable"));
                    let not_signed_in = p.error.as_ref().is_some_and(|e| e.kind == "not_configured");
                    card.hint = if not_signed_in { hint } else { String::new() };
                }
                Some(p) => {
                    card.connected = true;
                    card.stale = p.stale || p.fetched_at.is_some_and(|t| q.now - t > q.stale_after);
                    if p.stale {
                        if let Some(e) = &p.error {
                            card.error = output::sentence_case(&e.message);
                        }
                    }
                    let mut last: Option<String> = None;
                    for l in &p.limits {
                        let title = l.account_label.clone().unwrap_or_default();
                        let header = if last.as_deref() != Some(title.as_str()) {
                            last = Some(title.clone());
                            title
                        } else {
                            String::new()
                        };
                        card.lines.push(usage_line(&name, l, header, q));
                    }
                    if let Some(today) = today_line(id, q.report, q.now) {
                        card.lines.push(today);
                    }
                }
            }
            card
        })
        .collect()
}

/// True when no enabled provider has limits, so the home is the welcome.
pub fn show_empty(cards: &[UsageCard]) -> bool {
    !cards.iter().any(|c| c.connected)
}

/// One Priority row.
#[derive(Debug, Clone, PartialEq)]
pub struct PriorityRow {
    pub heading: String,
    pub logo: Logo,
    pub initial: String,
    pub title: String,
    pub subtitle: String,
    pub percent: String,
    pub reset: String,
    pub tone: i32,
    pub used_up: bool,
    pub track_path: String,
    pub arc_path: String,
    pub copy: String,
    pub a11y: String,
}

struct Candidate {
    logo: Logo,
    initial: String,
    provider: String,
    title: String,
    subtitle: String,
    left: f64,
    fill: f64,
    percent: String,
    reset: String,
    tone: i32,
    used_up: bool,
    copy: String,
}

fn pin_hit(pinned: &[String], provider: &str, label: &str) -> bool {
    pinned.iter().any(|k| k.split_once('/').is_some_and(|(p, l)| p.eq_ignore_ascii_case(provider) && l == label))
}

fn to_row(c: &Candidate, heading: &str) -> PriorityRow {
    let (track_path, arc_path) = if c.used_up {
        (String::new(), String::new())
    } else {
        let r = crate::arc::radius(32.0, 2.5);
        (crate::arc::circle(r), crate::arc::arc(c.fill, r))
    };
    let a11y = if c.used_up {
        format!("{}, {}, used up, {}", c.title, c.subtitle, c.reset)
    } else {
        format!("{}, {}, {}, {}", c.title, c.subtitle, c.percent, c.reset)
    };
    PriorityRow {
        heading: heading.to_string(),
        logo: c.logo,
        initial: c.initial.clone(),
        title: c.title.clone(),
        subtitle: c.subtitle.clone(),
        percent: if c.used_up { String::new() } else { c.percent.clone() },
        reset: c.reset.clone(),
        tone: c.tone,
        used_up: c.used_up,
        track_path,
        arc_path,
        copy: c.copy.clone(),
        a11y,
    }
}

/// Pinned first, then limits at or under the running-out threshold, then
/// limits that are used up. A pinned limit is not repeated lower down.
/// Healthy unpinned limits stay on All usage only.
pub fn priority_rows(snap: &Snapshot, enabled: &[String], q: &HomeQuery) -> Vec<PriorityRow> {
    let mut items = Vec::new();
    for id in enabled {
        let Some(p) = snap.provider(id) else { continue };
        if p.is_unavailable() {
            continue;
        }
        let name = popup_name(id);
        for l in &p.limits {
            if l.has_reset(q.now) {
                continue;
            }
            let left = l.percent_left();
            let (fill, _) = beside(l, q.mode);
            let up = used_up(l);
            items.push(Candidate {
                logo: logo_for(id),
                initial: initial(name),
                provider: id.clone(),
                title: l.label.clone(),
                subtitle: match l.account_label.as_deref() {
                    Some(plan) if !plan.is_empty() => format!("{name} · {plan}"),
                    _ => name.to_string(),
                },
                left,
                fill: fill.clamp(0.0, 1.0),
                percent: left_words(l, q.mode),
                reset: reset_phrase(l, q.now, q.timers),
                tone: tone_of(left, q.low_left, q.hot_left),
                used_up: up,
                copy: copy_of(name, &l.label, l, q.now, q.timers),
            });
        }
    }
    let pinned_of = |c: &Candidate| pin_hit(q.pinned, &c.provider, &c.title);
    let mut out = Vec::new();
    let mut first = true;
    for key in q.pinned {
        let Some((pid, label)) = key.split_once('/') else { continue };
        let Some(c) = items.iter().find(|c| c.provider.eq_ignore_ascii_case(pid) && c.title == label) else { continue };
        out.push(to_row(c, if first { "Pinned" } else { "" }));
        first = false;
    }
    let mut running: Vec<&Candidate> = items.iter().filter(|c| !c.used_up && c.left <= q.low_left && !pinned_of(c)).collect();
    running.sort_by(|a, b| a.left.total_cmp(&b.left));
    first = true;
    for c in running {
        out.push(to_row(c, if first { "Running out" } else { "" }));
        first = false;
    }
    first = true;
    for c in items.iter().filter(|c| c.used_up && !pinned_of(c)) {
        out.push(to_row(c, if first { "Used up" } else { "" }));
        first = false;
    }
    out
}

// ---------------------------------------------------------------------------
// Add account

#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    pub id: &'static str,
    pub logo: Logo,
    pub initial: String,
    pub name: String,
    /// Short name for the Add account page (`ChatGPT`, `Claude`).
    pub label: String,
    pub state: String,
    pub how: String,
    pub connected: bool,
}

/// Every provider Quota can meter, with its state from the last refresh
/// and how to connect it. Quota has no login of its own: connecting means
/// signing in to the tool, whose saved sign-in Quota then reads.
/// `enabled`: the providers turned on in Settings (None = all of them).
pub fn connectors(snap: &Snapshot, enabled: Option<&[String]>) -> Vec<Connector> {
    const LIST: [(&str, &str, &str); 5] = [
        ("claude", "Claude Code", "Run `claude` in a terminal and sign in."),
        ("codex", "Codex", "Run `codex login` in a terminal."),
        ("cursor", "Cursor", "Sign in inside Cursor."),
        ("copilot", "GitHub Copilot", "Sign in to GitHub Copilot in your editor."),
        ("openrouter", "OpenRouter", "Set OPENROUTER_API_KEY, or sign in to OpenRouter in OpenCode."),
    ];
    LIST.iter()
        .map(|(id, name, how)| {
            if enabled.is_some_and(|e| !e.iter().any(|x| x == id)) {
                return Connector {
                    id,
                    logo: logo_for(id),
                    initial: initial(name),
                    name: (*name).to_string(),
                    label: popup_name(id).to_string(),
                    state: "Turned off".into(),
                    how: "Turn it on in Settings.".into(),
                    connected: false,
                };
            }
            let p = snap.provider(id);
            let connected = p.is_some_and(|p| !p.is_unavailable());
            let state = match p {
                None => "Not checked yet".to_string(),
                Some(p) if !p.is_unavailable() => "Connected".to_string(),
                Some(p) => match p.error.as_ref().map(|e| e.kind.as_str()) {
                    Some("not_configured") => "Not signed in".into(),
                    Some("auth") => "Sign-in expired".into(),
                    Some("network") => "Can't reach it".into(),
                    _ => "Not available".into(),
                },
            };
            let kind = p.and_then(|p| p.error.as_ref()).map(|e| e.kind.as_str());
            let how = if connected && *id == "openrouter" {
                "Using your OpenRouter API key on this PC.".to_string()
            } else if connected {
                format!("Using your {name} sign-in on this PC.")
            } else if let (Some(e), false) = (p.and_then(|p| p.error.as_ref()), kind == Some("not_configured")) {
                // Signed in but unusable: the provider's own short reason.
                output::sentence_case(&e.message)
            } else {
                (*how).to_string()
            };
            Connector { id, logo: logo_for(id), initial: initial(name), name: (*name).to_string(), label: popup_name(id).to_string(), state, how, connected }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Settings

/// Poll interval choices in Settings, in seconds.
pub const POLL_CHOICES: [u64; 4] = [180, 300, 600, 900];

/// (index into `POLL_CHOICES` or -1, note shown when it is not a choice).
pub fn poll_choice(secs: u64) -> (i32, String) {
    match POLL_CHOICES.iter().position(|c| *c == secs) {
        Some(i) => (i as i32, String::new()),
        None => (-1, format!("config.toml sets every {}.", format::duration(secs as i64))),
    }
}

/// System, Light, Dark -> 0, 1, 2.
pub fn theme_index(t: quota_core::config::Theme) -> i32 {
    use quota_core::config::Theme;
    match t {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderToggle {
    pub id: &'static str,
    pub logo: Logo,
    pub initial: String,
    pub name: String,
    pub state: String,
    pub enabled: bool,
}

/// One row per supported provider for the Settings page.
pub fn provider_toggles(snap: &Snapshot, enabled: &[String]) -> Vec<ProviderToggle> {
    connectors(snap, Some(enabled))
        .into_iter()
        .map(|c| {
            let on = enabled.iter().any(|e| e == c.id);
            let state = if on { c.state } else { "Off".to_string() };
            ProviderToggle { id: c.id, logo: c.logo, initial: c.initial, name: c.name, state, enabled: on }
        })
        .collect()
}

/// The enabled list after flipping `id`, in the canonical provider order.
pub fn toggled(enabled: &[String], id: &str) -> Vec<String> {
    let on = enabled.iter().any(|e| e == id);
    quota_core::providers::ALL_IDS
        .iter()
        .filter(|p| if **p == id { !on } else { enabled.iter().any(|e| e == *p) })
        .map(|p| p.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quota_core::ProviderError;

    const H: i64 = 3600;
    const LEFT: UsageMode = UsageMode::Left;
    const REL: ResetTimers = ResetTimers::Relative;

    fn limit(provider: &str, label: &str, used: f64, start: Option<i64>, end: Option<i64>) -> Limit {
        Limit {
            provider: provider.into(),
            account_label: Some("Pro".into()),
            label: label.into(),
            used,
            limit: 100.0,
            unit: Unit::Percent,
            window_start: start,
            resets_at: end,
        }
    }

    fn sample() -> Snapshot {
        let mut s = Snapshot::default();
        s.apply(
            "claude",
            "Claude Code",
            Ok(vec![
                // 50% after 1h of 5h: behind, runs out at 2h.
                limit("claude", "Session", 50.0, Some(0), Some(5 * H)),
                // 20% after 50h of 100h: ahead.
                limit("claude", "Weekly", 20.0, Some(-49 * H), Some(51 * H)),
            ]),
            H,
        );
        s.apply("codex", "Codex", Ok(vec![limit("codex", "Monthly", 18.0, Some(-17 * H), Some(83 * H))]), H);
        s.apply("cursor", "Cursor", Err(ProviderError::Unavailable("token is encrypted".into())), H);
        s
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn footer_and_welcome() {
        assert_eq!(footer(0, false, Some(300)), "Next update in 5m");
        assert_eq!(footer(0, true, Some(300)), "Updating…");
        assert_eq!(footer(0, false, None), "Paused");
        assert!(!is_welcome(&sample()));
        assert!(!is_welcome(&Snapshot::default()), "not before the first check");
        let mut none = Snapshot::default();
        none.apply("cursor", "Cursor", Err(ProviderError::Unavailable("x".into())), H);
        assert!(is_welcome(&none));
        assert_eq!(logo_for("grok"), Logo::None);
        assert_eq!(initial("Grok"), "G");
    }

    #[test]
    fn connector_states() {
        let c = connectors(&sample(), None);
        assert_eq!(c.len(), 5);
        assert_eq!((c[0].name.as_str(), c[0].state.as_str(), c[0].connected), ("Claude Code", "Connected", true));
        assert!(c[0].how.starts_with("Using your Claude Code sign-in"));
        assert_eq!((c[2].name.as_str(), c[2].state.as_str()), ("Cursor", "Not available"));
        assert_eq!(c[3].state, "Not checked yet", "Copilot is not in the sample snapshot");
        assert_eq!(c[2].how, "Token is encrypted", "an unusable sign-in shows the provider's reason");
    }

    #[test]
    fn settings_rows() {
        let on = ids(&["claude", "cursor"]);
        let t = provider_toggles(&sample(), &on);
        assert_eq!(t.len(), 5);
        assert_eq!((t[0].name.as_str(), t[0].state.as_str(), t[0].enabled), ("Claude Code", "Connected", true));
        assert_eq!((t[1].state.as_str(), t[1].enabled), ("Off", false));
        assert_eq!(t[2].state, "Not available", "Cursor is on but unusable");
        assert_eq!(toggled(&on, "codex"), vec!["claude", "codex", "cursor"], "canonical order");
        assert_eq!(toggled(&on, "claude"), vec!["cursor"]);
        assert_eq!(poll_choice(300), (1, String::new()));
        assert_eq!(poll_choice(240).0, -1);
        assert!(poll_choice(240).1.contains("4m"));
    }

    fn query(pinned: &[String]) -> HomeQuery<'_> {
        HomeQuery { now: H, stale_after: 360, mode: LEFT, timers: REL, low_left: 30.0, hot_left: 10.0, pinned, report: None }
    }

    #[test]
    fn home_pages_follow_the_menu() {
        let enabled = ids(&["codex", "claude", "cursor"]);
        let cards = usage_cards(&sample(), &enabled, &query(&[]));
        assert_eq!(cards.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["ChatGPT", "Claude", "Cursor"]);
        assert!(cards[0].connected && cards[0].lines[0].header == "Pro");
        assert_eq!(cards[0].lines[0].amount, "82%");
        assert_eq!(cards[0].lines[0].tone, 0);
        assert_eq!((cards[1].lines[0].label.as_str(), cards[1].lines[0].amount.as_str(), cards[1].lines[1].header.as_str()), ("Session", "50%", ""));
        assert_eq!(cards[1].lines[1].amount, "80%");
        assert!(!cards[2].connected && cards[2].error == "Token is encrypted");
        assert!(!show_empty(&cards));
        assert!(priority_rows(&sample(), &enabled, &query(&[])).is_empty(), "nothing pinned, low, or used up");
        assert!(show_empty(&usage_cards(&Snapshot::default(), &enabled, &query(&[]))));
        assert_eq!(level_bands(&[30.0, 10.0]), (30.0, 10.0));
        assert_eq!(level_bands(&[]), (30.0, 10.0));
        // Used mode flips the number and the fill. Weekly is 20% used.
        let mut used = query(&[]);
        used.mode = UsageMode::Used;
        let weekly = &usage_cards(&sample(), &ids(&["claude"]), &used)[0].lines[1];
        assert_eq!(weekly.amount, "20%");
        assert!((weekly.fill - 0.2).abs() < 1e-6);
        let usd = Limit { unit: Unit::Usd, limit: 20.0, used: 5.0, ..limit("openrouter", "Credits", 0.0, None, Some(10 * H)) };
        let mut snap = Snapshot::default();
        snap.apply("openrouter", "OpenRouter", Ok(vec![usd]), H);
        assert_eq!(usage_cards(&snap, &ids(&["openrouter"]), &query(&[]))[0].lines[0].amount, "$15.00");
    }

    #[test]
    fn priority_sections_and_groups() {
        let mut session = limit("claude", "Session", 100.0, Some(0), Some(5 * H));
        session.account_label = Some("Personal".into());
        let mut weekly = limit("claude", "Weekly", 25.0, Some(0), Some(100 * H));
        weekly.account_label = Some("Work".into());
        let mut monthly = limit("codex", "Monthly", 89.0, Some(0), Some(100 * H));
        monthly.account_label = Some("Plus".into());
        let mut snap = Snapshot::default();
        snap.apply("claude", "Claude Code", Ok(vec![session, weekly]), H);
        snap.apply("codex", "Codex", Ok(vec![monthly]), H);
        let pinned = ids(&["claude/Weekly"]);
        let rows = priority_rows(&snap, &ids(&["claude", "codex"]), &query(&pinned));
        assert_eq!(rows.iter().map(|r| r.heading.as_str()).collect::<Vec<_>>(), ["Pinned", "Running out", "Used up"]);
        assert_eq!((rows[0].title.as_str(), rows[0].percent.as_str(), rows[0].subtitle.as_str()), ("Weekly", "75% left", "Claude · Work"));
        assert!(!rows[0].used_up && !rows[0].track_path.is_empty() && rows[0].arc_path.contains('A'));
        assert_eq!((rows[1].title.as_str(), rows[1].percent.as_str(), rows[1].tone), ("Monthly", "11% left", 1));
        assert!(rows[2].used_up && rows[2].percent.is_empty() && rows[2].arc_path.is_empty());
        assert_eq!(rows[2].title, "Session");
        // Two account labels are two groups, and a real token total is a Today line.
        let now = quota_core::time::parse_rfc3339("2026-11-12T12:00:00Z").unwrap();
        let today = quota_core::tokens::local_day(now);
        let mut report = Report::default();
        report.claude.files = 1;
        report.claude.days.insert(today, 2_400_000);
        let q = HomeQuery { now, report: Some(&report), ..query(&[]) };
        let lines = &usage_cards(&snap, &ids(&["claude"]), &q)[0].lines;
        assert_eq!((lines[0].header.as_str(), lines[1].header.as_str(), lines[2].label.as_str()), ("Personal", "Work", "Today"));
        assert_eq!((lines[2].kind, lines[2].amount.as_str()), (1, "2.4M tokens"));
        // Files exist, but nothing was counted today: no Today row.
        report.codex.files = 1;
        let q0 = HomeQuery { now, report: Some(&report), ..query(&[]) };
        let codex = &usage_cards(&snap, &ids(&["codex"]), &q0)[0].lines;
        assert!(codex.iter().all(|l| l.label != "Today"));
        // 7% left is red, not orange.
        assert_eq!(tone_of(7.0, 30.0, 10.0), 2);
        assert_eq!(tone_of(18.0, 30.0, 10.0), 1);
    }
}
