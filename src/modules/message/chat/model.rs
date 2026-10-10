//! The chat transcript and its threads, from the stored messages (flick-1a7e).
//!
//! - `transcript`: one thread's messages (oldest first, as `Messages::thread` returns them)
//!   as rows: a divider at each new local day, a bubble per text message (the user's on the
//!   right, KOTA's on the left), a card row per stored card, and a "Thinking…" bubble while
//!   the newest question has no answer yet (for at most `THINKING_SECS`).
//! - States: a 'me' message is `Done` or `Failed` (an ask that did not go out; ⌘R retries
//!   it, `retry`), never pending; a peer post is `Pending` (`--pending`), `Streaming`
//!   (`--partial`), `Failed` or `Done`.
//! - `version` changes whenever anything a row shows changes, so the surface rebuilds only
//!   those rows; `fold` mixes in what the module adds (a card's press state).
//! - Threads: `title` (the first question, clipped), `neighbor` (⌘[ older, ⌘] newer),
//!   `status` for the header, `new_thread_id` (`t` + base36 time, a valid card id).
//! - `alert`: whether a post shows a HUD card and plays a sound, given the thread the chat
//!   window shows.
//!
//! The row types mirror `platform::surface::Row`; modules cannot import each other, and the
//! wiring maps these onto the surface's.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::card::Card;
use crate::modules::message::card;
use crate::modules::message::store::{Message, Progress, Role, Thread};
use crate::modules::message::text;
use crate::platform::surface::rows::message_text;

/// Seconds a question shows "Thinking…" without an answer; after that KOTA is taken to
/// have dropped it and the bubble goes.
pub const THINKING_SECS: i64 = 600;
/// What the thinking bubble says.
pub const THINKING: &str = "Thinking…";
/// The header of a question that did not go out.
pub const NOT_SENT: &str = "Not sent · ⌘R retries";
/// Characters of a thread title.
pub const TITLE_MAX: usize = 48;
/// The title of a thread with no text yet.
pub const UNTITLED: &str = "New chat";

/// Whose a bubble is: the user's (right) or the peer's (left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Mine,
    Theirs,
}

/// Where a bubble's text is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Done,
    /// A reply still arriving (`post --partial`).
    Streaming,
    /// Waiting for its text: a `--pending` post, or the thinking bubble.
    Pending,
    /// A question that did not reach KOTA, or a failed post.
    Failed,
}

/// One transcript row; `platform::surface::Row` without the borrowing.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// Markdown-lite text. `key` is the message id (`thinking:<id>` for the thinking bubble).
    Bubble { key: String, version: u64, side: Side, header: String, time: String, md: String, state: State },
    /// A stored card; the wiring adds its press state and folds it into `version`.
    Card { key: String, version: u64, card: Box<Card> },
    /// A day caption ("Today", "Yesterday", "Oct 9").
    Divider { text: String },
}

/// The header's state: busy while KOTA works on the thread, error when the last question
/// did not go out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Busy,
    Error,
}

/// Which way ⌘[ / ⌘] moves through the threads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// ⌘[: the thread with the next older last message.
    Older,
    /// ⌘]: the next newer one.
    Newer,
}

/// Whether a post shows a HUD card and plays a sound (the sound still needs `[message] sound`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alert {
    pub hud: bool,
    pub sound: bool,
}

/// The time the transcript is drawn at: now, and the local offset of a time (seconds east
/// of UTC).
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub now: i64,
    pub offset: fn(i64) -> i32,
}

/// The rows of one thread, `list` oldest first. `peer` heads a peer's bubble when its post
/// has no `--title` (`[message] name`, or "KOTA").
pub fn transcript(list: &[Message], peer: &str, clock: Clock) -> Vec<Row> {
    let mut rows = vec![];
    let mut shown_day = None;
    for m in list {
        let offset = (clock.offset)(m.ts);
        let day = (m.ts + i64::from(offset)).div_euclid(86_400);
        if shown_day != Some(day) {
            rows.push(Row::Divider { text: text::day(m.ts, clock.now, offset) });
            shown_day = Some(day);
        }
        rows.push(row(m, peer, offset));
    }
    if let Some(q) = waiting(list, clock.now) {
        rows.push(Row::Bubble {
            key: format!("thinking:{}", q.id),
            version: 0,
            side: Side::Theirs,
            header: peer.to_string(),
            time: String::new(),
            md: THINKING.into(),
            state: State::Pending,
        });
    }
    rows
}

/// One message's row: its card if it holds one that still parses, else a bubble.
fn row(m: &Message, peer: &str, offset: i32) -> Row {
    let version = version(m);
    if let Some(c) = card::stored(m) {
        return Row::Card { key: m.id.clone(), version, card: Box::new(c) };
    }
    let (side, header, state) = match m.role {
        Role::Me if m.state == Progress::Failed => (Side::Mine, NOT_SENT.to_string(), State::Failed),
        Role::Me => (Side::Mine, String::new(), State::Done),
        Role::Peer => {
            let state = match m.state {
                _ if m.pending => State::Pending,
                Progress::Partial => State::Streaming,
                Progress::Failed => State::Failed,
                Progress::Done => State::Done,
            };
            (Side::Theirs, m.title.clone().unwrap_or_else(|| peer.to_string()), state)
        }
    };
    let time = text::stamp(m.ts, m.ts, offset);
    Row::Bubble { key: m.id.clone(), version, side, header, time, md: m.body.clone(), state }
}

/// The question still waiting for KOTA: the newest message is the user's, it went out, and
/// it is under `THINKING_SECS` old.
fn waiting(list: &[Message], now: i64) -> Option<&Message> {
    let q = list.last().filter(|m| m.role == Role::Me && m.state == Progress::Done)?;
    (now - q.ts < THINKING_SECS).then_some(q)
}

/// A hash of everything a message's row shows.
pub fn version(m: &Message) -> u64 {
    let progress = match m.state {
        Progress::Done => 0u8,
        Progress::Partial => 1,
        Progress::Failed => 2,
    };
    let mut h = DefaultHasher::new();
    (&m.body, &m.title, &m.card, m.ts, m.pending, m.role == Role::Me, progress).hash(&mut h);
    h.finish()
}

/// `version` with `part` mixed in (e.g. the `Debug` text of a card's press state).
pub fn fold(version: u64, part: &impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    (version, part).hash(&mut h);
    h.finish()
}

/// The header status of a thread, `list` oldest first.
pub fn status(list: &[Message], now: i64) -> Status {
    let streaming = list.iter().any(|m| m.role == Role::Peer && (m.pending || m.state == Progress::Partial));
    if streaming || waiting(list, now).is_some() {
        return Status::Busy;
    }
    match list.iter().rfind(|m| m.role == Role::Me) {
        Some(q) if q.state == Progress::Failed => Status::Error,
        _ => Status::Idle,
    }
}

/// The question ⌘R sends again: the newest 'me' message, if it failed.
pub fn retry(list: &[Message]) -> Option<&Message> {
    list.iter().rfind(|m| m.role == Role::Me).filter(|m| m.state == Progress::Failed)
}

/// The text ⌘⇧C copies: the newest peer reply in `list` (oldest first) with text, as its
/// bubble's Copy Message would (`surface::rows::message_text`). Cards and posts still
/// waiting for their text are skipped; a reply still streaming gives what has arrived.
pub fn last_reply(list: &[Message]) -> Option<&str> {
    list.iter()
        .rev()
        .filter(|m| m.role == Role::Peer && !m.pending && card::stored(m).is_none())
        .map(|m| message_text(&m.body))
        .find(|text| !text.is_empty())
}

/// A thread's title: its first message (`Thread::first_body`) on one line, clipped.
pub fn title(first_body: &str) -> String {
    let t = text::preview(&text::plain(first_body), TITLE_MAX);
    if t.is_empty() { UNTITLED.into() } else { t }
}

/// The thread ⌘[ or ⌘] moves to from `current`; `threads` newest first, as
/// `Messages::threads` lists them. From a thread not in the list (a new one, still empty)
/// ⌘[ goes to the newest stored thread. `None` at either end.
pub fn neighbor<'a>(threads: &'a [Thread], current: Option<&str>, step: Step) -> Option<&'a str> {
    let at = current.and_then(|c| threads.iter().position(|t| t.id == c));
    let to = match (at, step) {
        (None, Step::Older) => Some(0),
        (None, Step::Newer) => None,
        (Some(i), Step::Older) => Some(i + 1),
        (Some(i), Step::Newer) => i.checked_sub(1),
    };
    to.and_then(|i| threads.get(i)).map(|t| t.id.as_str())
}

/// Whether post `m` shows a HUD card and sounds. `first`: no message with its id was stored
/// before it. `open`: the thread the chat window shows, `None` when it is hidden.
///
/// The user's own messages never alert. A threaded post alerts not at all while the window
/// shows its thread; otherwise its HUD card shows on the first post of the id and the final
/// one (not `--pending`, not `--partial`), and it sounds only on the final one. Unthreaded
/// posts keep the HUD's behaviour: every post shows, placeholders and partial posts are
/// silent.
pub fn alert(m: &Message, first: bool, open: Option<&str>) -> Alert {
    let done = !m.pending && m.state != Progress::Partial;
    match &m.thread {
        _ if m.role == Role::Me => Alert { hud: false, sound: false },
        Some(t) if open == Some(t.as_str()) => Alert { hud: false, sound: false },
        Some(_) => Alert { hud: first || done, sound: done },
        None => Alert { hud: true, sound: done },
    }
}

/// A fresh thread id: `t`, the time in ms and a counter, base 36 (a valid card id).
pub fn new_thread_id() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    thread_id(ms, N.fetch_add(1, Ordering::Relaxed))
}

fn thread_id(ms: u128, n: u32) -> String {
    format!("t{}{}", base36(ms), base36(u128::from(n % 36)))
}

fn base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = String::new();
    loop {
        out.insert(0, char::from(DIGITS[(n % 36) as usize]));
        n /= 36;
        if n == 0 {
            return out;
        }
    }
}

#[cfg(test)]
mod tests;
