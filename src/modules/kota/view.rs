//! What KOTA's presence looks like, as plain data (100% coverage floor): the menu bar
//! title and tooltip, the menu entries (flick-78b9 renders them through
//! `platform::status_item`), `kota status` as text and JSON, and the ask view's text.

use serde_json::{Value, json};

use super::ask::{Ask, Status};
use super::presence::{Presence, State};

/// One menu row. `Pick` calls the module's pick handler with `key`; `Open` routes `key`
/// like a press of the module's hotkey.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "flick-78b9 renders the menu"))]
pub enum Entry {
    Info(String),
    Separator,
    Pick { title: String, key: &'static str },
    Open { title: String, key: &'static str },
}

/// The menu bar glyph: a `K` and a mark for the state; `K?` while stale.
#[cfg_attr(not(test), expect(dead_code, reason = "flick-78b9 renders the title"))]
pub fn glyph(p: &Presence) -> &'static str {
    if p.stale {
        return "K?";
    }
    match p.state {
        State::Thinking => "K…",
        State::Blocked => "K!",
        State::Idle => "K",
        State::Degraded => "K~",
        State::Down => "K×",
        State::Offline => "K-",
        State::Unknown => "K?",
    }
}

/// The menu bar title: the glyph, plus the count of cards waiting on the user.
#[cfg_attr(not(test), expect(dead_code, reason = "flick-78b9 renders the title"))]
pub fn title(p: &Presence, pending: u32) -> String {
    match pending {
        0 => glyph(p).to_string(),
        n => format!("{} {n}", glyph(p)),
    }
}

/// How long ago `since` was, short: `<1m`, `4m`, `3h`, `2d`.
pub fn age(since: u64, now: u64) -> String {
    let secs = now.saturating_sub(since);
    match secs {
        0..60 => "<1m".into(),
        60..3_600 => format!("{}m", secs / 60),
        3_600..86_400 => format!("{}h", secs / 3_600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// `HH:MM` local time for unix `at`, `offset` seconds east of UTC.
pub fn clock(at: u64, offset: i32) -> String {
    let local = i64::try_from(at).unwrap_or(i64::MAX).saturating_add(i64::from(offset));
    let day = local.rem_euclid(86_400);
    format!("{:02}:{:02}", day / 3_600, day % 3_600 / 60)
}

/// `KOTA: thinking · 4m`; `(stale)` after a sleep until a round confirms it.
pub fn headline(p: &Presence, now: u64) -> String {
    let mut line = format!("KOTA: {}", p.state.word());
    if let Some(since) = p.since {
        line = format!("{line} · {}", age(since, now));
    }
    if p.stale {
        line.push_str(" (stale)");
    }
    line
}

/// The tooltip: the headline and KOTA's current task.
#[cfg_attr(not(test), expect(dead_code, reason = "flick-78b9 renders the tooltip"))]
pub fn tooltip(p: &Presence, now: u64) -> String {
    match p.seen.pane.as_ref().filter(|pane| !pane.title.is_empty()) {
        Some(pane) => format!("{}\n{}", headline(p, now), pane.title),
        None => headline(p, now),
    }
}

/// The menu's information rows: headline, pane title, failing checks, last check.
/// `offset`: the local UTC offset at `now`.
pub fn info(p: &Presence, now: u64, offset: i32) -> Vec<String> {
    let mut rows = vec![headline(p, now)];
    if let Some(pane) = p.seen.pane.as_ref().filter(|pane| !pane.title.is_empty()) {
        rows.push(pane.title.clone());
    }
    if !p.seen.failing.is_empty() {
        rows.push(format!("Checks: {} failing", p.seen.failing.join(", ")));
    }
    rows.push(match (p.stale, p.checked_at) {
        (true, _) => "stale".into(),
        (false, Some(at)) => format!("Checked {}", clock(at, offset)),
        (false, None) => "Not checked yet".into(),
    });
    rows
}

/// The whole menu, top to bottom.
#[cfg_attr(not(test), expect(dead_code, reason = "flick-78b9 renders the menu"))]
pub fn menu(p: &Presence, pending: u32, now: u64, offset: i32) -> Vec<Entry> {
    let mut entries: Vec<Entry> = info(p, now, offset).into_iter().map(Entry::Info).collect();
    let inbox = match pending {
        0 => "Inbox".to_string(),
        n => format!("Inbox ({n} waiting)"),
    };
    entries.extend([
        Entry::Separator,
        Entry::Open { title: "Ask KOTA…".into(), key: "ask" },
        Entry::Open { title: inbox, key: "inbox" },
        Entry::Pick { title: "Open Dashboard".into(), key: "dash" },
        Entry::Pick { title: "Refresh Now".into(), key: "refresh" },
    ]);
    entries
}

/// How rounds run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polling {
    /// No `[kota]` key set: rounds run only on `kota refresh`.
    Off,
    /// `poll_secs = 0`: rounds run only on demand.
    OnDemand,
    /// Every `poll_secs`, faster while KOTA works.
    Every(u64),
}

impl Polling {
    pub fn text(self) -> String {
        match self {
            Polling::Off => "off (set a key in [kota] to poll)".into(),
            Polling::OnDemand => "on demand".into(),
            Polling::Every(secs) => format!("every {secs} s"),
        }
    }
}

/// Facts for `kota status` besides the presence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extra {
    /// Cards waiting on the user (`Event::CardsPending`).
    pub pending: u32,
    pub polling: Polling,
}

/// `kota status --json`.
pub fn status_json(p: &Presence, extra: Extra) -> Value {
    let pane = p.seen.pane.as_ref().map(|pane| {
        json!({ "id": pane.id, "name": pane.name, "status": pane.status, "title": pane.title })
    });
    json!({
        "state": p.state.word(),
        "stale": p.stale,
        "since": p.since,
        "pane": pane,
        "failing": p.seen.failing,
        "errors": p.errors,
        "checked_at": p.checked_at,
        "pending": extra.pending,
        "polling": extra.polling.text(),
    })
}

/// `kota status`: the menu's information rows, the errors, and how polling runs.
pub fn status_text(p: &Presence, extra: Extra, now: u64, offset: i32) -> String {
    let mut rows = info(p, now, offset);
    rows.extend(p.errors.iter().cloned());
    if extra.pending > 0 {
        rows.push(format!("{} waiting", extra.pending));
    }
    rows.push(format!("polling: {}", extra.polling.text()));
    rows.join("\n")
}

/// The ask view's text: the remembered asks, newest first, one line each:
/// `12:03  sent  what's on today?`. `offset`: the local UTC offset.
pub fn asks_text(asks: &[Ask], offset: i32) -> String {
    let line = |a: &Ask| {
        let status = match &a.status {
            Status::Sending => "sending…".to_string(),
            Status::Sent(_) => "sent".to_string(),
            Status::Failed(why) => format!("failed: {why}"),
        };
        let text: String = a.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let text = match text.char_indices().nth(80) {
            Some((at, _)) => format!("{}…", &text[..at]),
            None => text,
        };
        format!("{}  {status}  {text}", clock(a.at, offset))
    };
    asks.iter().map(line).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests;
