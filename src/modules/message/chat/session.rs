//! The KOTA chat window, wired (flick-eedd): summon and hide, the thread it shows, its keys,
//! and redraws from the store. Asks are in `asks.rs`, the launcher's thread list in
//! `threads.rs`, the row mapping in `view.rs`; the real hooks in `wire.rs`.
//!
//! - `[message] chat_hotkey` toggles the window (`platform::surface` "chat"): hidden, or shown
//!   without the keyboard, it shows and takes the keyboard; shown with it, it hides. Flick
//!   never activates, so the app in front stays active. The pid in front at summon is kept,
//!   and on Esc, ⌘W or the hotkey hiding the window that app is made frontmost again only if
//!   it still is (another app may have taken over meanwhile).
//! - The thread shown: the one asked for (`message chat --thread t`, the launcher's
//!   `message/threads`), else the last shown, else the newest stored, else a new one (`t…`,
//!   stored once the first question is). ⌘N starts a thread, ⌘[ / ⌘] move to the older /
//!   newer one (`model::neighbor`), ⌘R resends the newest question that did not go out.
//! - Every surface handler only queues a `Note` and posts `ModuleChanged` (mx-fcbc43); the
//!   module drains the notes on the main thread (`chat_drain`), then redraws the window if it
//!   shows. Card presses go through the HUD cards' dispatch (`dispatch.rs`).

use std::collections::VecDeque;

use super::asks::{Ask, Asked};
use super::{model, view};
use crate::core::Cx;
use crate::modules::message::Inbox;
use crate::modules::message::run::{self, Worker};
use crate::modules::message::store::Messages;
use crate::platform::surface::{self, Key, Keystroke};

/// What the window needs from the system; `wire::HOOKS` is the real thing.
#[derive(Clone, Copy)]
pub struct Hooks {
    /// Build the window (once; later calls do nothing).
    pub open: fn(),
    /// Show it on the screen under the pointer with the keyboard.
    pub show: fn(),
    /// Hide it without a `Closed` note.
    pub hide: fn(),
    pub visible: fn() -> bool,
    /// Whether it has the keyboard.
    pub key: fn() -> bool,
    /// Title, subtitle, status dot.
    pub header: fn(&str, &str, surface::Status),
    pub rows: fn(&[surface::Row]),
    pub notice: fn(Option<&str>),
    pub set_input: fn(&str),
    /// The notes the window's handlers queued.
    pub take: fn() -> Vec<Note>,
    /// The pid of the app in front.
    pub front: fn() -> Option<i32>,
    /// Make app `pid` frontmost again.
    pub refocus: fn(i32),
    /// Run one ask (ssh to kota-ask) on the worker thread.
    pub ask: fn(&run::Job) -> run::Exit,
    /// A fresh thread id (`model::new_thread_id`).
    pub new_thread: fn() -> String,
    /// Draw the window into a PNG at a path (`surface::snapshot`).
    pub snapshot: fn(&str) -> Result<(), String>,
}

/// What a window handler queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// Return with text in the input (the input is already empty).
    Submit(String),
    /// A chat key.
    Key(Command),
    /// The user hid the window (Esc).
    Closed,
    /// A card row's button: card id, action id, values JSON.
    Press { card: String, action: String, values: String },
}

/// The chat's own ⌘ keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// ⌘N
    New,
    /// ⌘[
    Older,
    /// ⌘]
    Newer,
    /// ⌘R
    Retry,
    /// ⌘W
    Close,
}

/// The command bound to `k`, if any: ⌘ alone with N, [, ], R or W.
pub fn binding(k: Keystroke) -> Option<Command> {
    if !k.cmd || k.shift || k.opt {
        return None;
    }
    match k.key {
        Key::Char('n') => Some(Command::New),
        Key::Char('[') => Some(Command::Older),
        Key::Char(']') => Some(Command::Newer),
        Key::Char('r') => Some(Command::Retry),
        Key::Char('w') => Some(Command::Close),
        _ => None,
    }
}

/// The chat window's state in the module.
pub struct Chat {
    pub hooks: Hooks,
    /// The window was built.
    pub(super) opened: bool,
    /// The thread it shows.
    pub(super) thread: Option<String>,
    /// The app in front when it was summoned.
    pub(super) front: Option<i32>,
    /// Asks waiting for the one in flight to end, first in first out.
    pub(super) queue: VecDeque<Ask>,
    /// The request id of the ask in flight.
    pub(super) busy: Option<String>,
    /// Finished asks, from the worker thread.
    pub(super) done: Worker<Asked>,
    /// The window's notice line (why the last ask failed).
    pub(super) notice: Option<String>,
}

impl Default for Chat {
    fn default() -> Self {
        Chat::new(super::wire::HOOKS)
    }
}

impl Chat {
    pub fn new(hooks: Hooks) -> Chat {
        Chat {
            hooks,
            opened: false,
            thread: None,
            front: None,
            queue: VecDeque::new(),
            busy: None,
            done: Worker::default(),
            notice: None,
        }
    }

    /// Whether the window shows.
    fn visible(&self) -> bool {
        self.opened && (self.hooks.visible)()
    }

    /// Give the keyboard back: the app in front at summon becomes frontmost again if it
    /// still is in front.
    fn refocus(&mut self) {
        if let Some(pid) = self.front.take()
            && (self.hooks.front)() == Some(pid)
        {
            (self.hooks.refocus)(pid);
        }
    }
}

impl Inbox {
    /// The thread the chat window shows, `None` while it is hidden (`model::alert`).
    pub fn chat_showing(&self) -> Option<&str> {
        self.chat.visible().then_some(self.chat.thread.as_deref()).flatten()
    }

    /// The hotkey: show the window (and take the keyboard), or hide it when it has it.
    pub fn chat_toggle(&mut self, cx: &Cx) {
        if self.chat.visible() && (self.chat.hooks.key)() {
            (self.chat.hooks.hide)();
            self.chat.refocus();
        } else {
            self.summon(None, cx);
        }
    }

    /// Show the window on `thread` (else the one it showed, the newest, or a new one).
    pub fn summon(&mut self, thread: Option<String>, cx: &Cx) {
        if !self.chat.visible() {
            self.chat.front = (self.chat.hooks.front)();
        }
        if thread.is_some() {
            self.chat.thread = thread;
        }
        self.current_thread(cx);
        if !self.chat.opened {
            (self.chat.hooks.open)();
            self.chat.opened = true;
        }
        self.chat_draw(cx);
        (self.chat.hooks.show)();
    }

    /// The thread the window shows, picking one when none is yet.
    pub fn current_thread(&mut self, cx: &Cx) -> String {
        if self.chat.thread.is_none() {
            let newest = cx.store.threads(1).pop().map(|t| t.id);
            self.chat.thread = Some(newest.unwrap_or_else(self.chat.hooks.new_thread));
        }
        self.chat.thread.clone().unwrap_or_default()
    }

    /// Handle what the window queued and the asks that ended, then redraw if it shows.
    pub fn chat_drain(&mut self, cx: &Cx) {
        for note in (self.chat.hooks.take)() {
            match note {
                Note::Submit(text) => {
                    let thread = self.current_thread(cx);
                    if let Err(e) = self.ask(&thread, &text, false, cx) {
                        (self.chat.hooks.set_input)(&text);
                        self.chat.notice = Some(e);
                    }
                }
                Note::Key(c) => self.chat_key(c, cx),
                Note::Closed => self.chat.refocus(),
                Note::Press { card, action, values } => {
                    // Errors show on the card.
                    let _ = self.press(&card, &action, values, cx);
                }
            }
        }
        for done in self.chat.done.take() {
            self.asked(done, cx);
        }
        self.chat_refresh(cx);
    }

    fn chat_key(&mut self, c: Command, cx: &Cx) {
        let step = match c {
            Command::New => {
                self.chat.thread = Some((self.chat.hooks.new_thread)());
                self.chat.notice = None;
                return;
            }
            Command::Retry => return self.retry(cx),
            Command::Close => {
                (self.chat.hooks.hide)();
                return self.chat.refocus();
            }
            Command::Older => model::Step::Older,
            Command::Newer => model::Step::Newer,
        };
        let threads = cx.store.threads(self.settings.chat_threads);
        if let Some(t) = model::neighbor(&threads, self.chat.thread.as_deref(), step) {
            self.chat.thread = Some(t.to_string());
            self.chat.notice = None;
        }
    }

    /// Redraw the window if it shows.
    pub fn chat_refresh(&self, cx: &Cx) {
        if self.chat.visible() {
            self.chat_draw(cx);
        }
    }

    /// `message chat --snapshot <path>`: draw the window, shown or not, into a PNG, so its
    /// rendering can be checked without Screen Recording (mx-6b45b0).
    pub fn chat_snapshot(&self, path: &str, cx: &Cx) -> Result<String, String> {
        if !self.chat.opened {
            return Err("The chat window was not opened yet".into());
        }
        self.chat_draw(cx);
        (self.chat.hooks.snapshot)(path)?;
        Ok(format!("Wrote {path}"))
    }

    /// Draw the current thread: header, rows, notice.
    fn chat_draw(&self, cx: &Cx) {
        let Some(thread) = self.chat.thread.as_deref() else { return };
        let list = cx.store.thread(thread, self.settings.chat_history);
        let clock = model::Clock { now: (self.env.now)(), offset: self.env.utc_offset };
        let rows = model::transcript(&list, &self.settings.name, clock);
        (self.chat.hooks.rows)(&view::rows(&rows, &self.ui));
        let title = list.first().map_or_else(|| model::UNTITLED.to_string(), |m| model::title(&m.body));
        let status = model::status(&list, clock.now);
        (self.chat.hooks.header)(&title, view::subtitle(status), view::status(status));
        (self.chat.hooks.notice)(self.chat.notice.as_deref());
    }
}

#[cfg(test)]
mod tests;
