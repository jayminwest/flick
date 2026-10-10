//! The normal chat window (flick-6a0d): summon and hide, the thread it shows, sending a
//! prompt, the streamed reply, stop, and history. The real hooks are `wire::UI`; tests use
//! `testkit::UI`, so no test opens a window.
//!
//! - `[llm] hotkey` (or Enter on the root item) shows the window (`platform::surface` "llm")
//!   with the keyboard; the hotkey hides it again when it has the keyboard. Flick never
//!   activates. The app in front at summon is made frontmost again when the window hides, if
//!   it still is in front.
//! - The thread shown: the one asked for (the `threads` view), else the one shown last, else
//!   the newest stored (with `history`), else a new one. Its server and model: the last
//!   picked in the `models` view, else `default_server` and `default_model`, else the first
//!   normal server and the first model it lists (fetched once; a prompt sent before the list
//!   lands waits for it).
//! - Return sends: the prompt and the whole thread (and `system_prompt`) go to `io::chat`.
//!   Each `ModuleChanged` takes the stream's new pieces; the reply shows as it arrives and is
//!   kept once it ends (done, stopped or failed). One reply at a time.
//! - Keys: ⌘N new chat, ⌘. stop the reply (what arrived stays), ⌘W hide. Esc hides too.
//! - History (`history = true`): each prompt and each ended reply is written to
//!   `llm_threads`/`llm_messages` (`store.rs`), the oldest threads past `max_threads`
//!   dropped. With `history = false` nothing is written and the thread lives only in memory.
//! - Every surface handler only queues a `Note` and posts `ModuleChanged` (mx-fcbc43); the
//!   module drains the notes on the main thread, then redraws the window if it shows.

use super::io::{self, Status};
use super::openai::{self, Piece, Role, Turn};
use super::store::{Chats, End, Msg, Save, Who};
use super::view::{self, Live, Phase};
use super::{Llm, settings::Server};
use crate::core::Cx;
use crate::platform::surface::{self, Key, Keystroke};

/// What the window needs from the system; `wire::UI` is the real thing.
#[derive(Clone, Copy)]
pub struct Ui {
    /// Build the window (once; later calls do nothing).
    pub open: fn(),
    /// Show it on the screen under the pointer with the keyboard.
    pub show: fn(),
    /// Hide it without a `Closed` note.
    pub hide: fn(),
    pub visible: fn() -> bool,
    /// Whether it has the keyboard.
    pub key: fn() -> bool,
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
    /// Unix seconds now.
    pub now: fn() -> i64,
    /// A fresh thread id (`thread_id`).
    pub new_id: fn() -> String,
}

/// What a window handler queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// Return with text in the input (the input is already empty).
    Submit(String),
    Key(Command),
    /// The user hid the window (Esc).
    Closed,
}

/// The chat's own ⌘ keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// ⌘N
    New,
    /// ⌘.
    Stop,
    /// ⌘W
    Close,
}

/// The command bound to `k`: ⌘ alone with N, . or W.
pub fn binding(k: Keystroke) -> Option<Command> {
    if !k.cmd || k.shift || k.opt {
        return None;
    }
    match k.key {
        Key::Char('n') => Some(Command::New),
        Key::Char('.') => Some(Command::Stop),
        Key::Char('w') => Some(Command::Close),
        _ => None,
    }
}

/// A thread id: `l`, the time in base-36 milliseconds, and a counter.
pub fn thread_id(ms: u128, n: u32) -> String {
    let mut digits = vec![];
    let mut v = ms;
    loop {
        let d = u32::try_from(v % 36).unwrap_or(0);
        digits.push(char::from_digit(d, 36).unwrap_or('0'));
        v /= 36;
        if v == 0 {
            break;
        }
    }
    let t: String = digits.into_iter().rev().collect();
    format!("l{t}{n}")
}

/// The thread the window shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub server: String,
    /// Empty until resolved (`default_model`, or the first one the server lists).
    pub model: String,
    pub msgs: Vec<Msg>,
}

/// A reply in flight: its stream and what arrived.
struct Reply {
    stream: u64,
    live: Live,
}

/// The chat window's state in the module.
pub struct Chat {
    pub ui: Ui,
    opened: bool,
    front: Option<i32>,
    pub(super) thread: Option<Thread>,
    /// The server and model picked last in the `models` view.
    pub(super) pick: Option<(String, String)>,
    reply: Option<Reply>,
    /// A prompt waiting for the server's model list.
    waiting: Option<String>,
    notice: Option<String>,
}

impl Chat {
    pub fn new(ui: Ui) -> Chat {
        Chat { ui, opened: false, front: None, thread: None, pick: None, reply: None, waiting: None, notice: None }
    }

    fn visible(&self) -> bool {
        self.opened && (self.ui.visible)()
    }

    fn refocus(&mut self) {
        if let Some(pid) = self.front.take()
            && (self.ui.front)() == Some(pid)
        {
            (self.ui.refocus)(pid);
        }
    }

    fn phase(&self) -> Phase {
        match (&self.reply, &self.waiting) {
            (Some(_), _) => Phase::Streaming,
            (None, Some(_)) => Phase::Loading,
            (None, None) => Phase::Idle,
        }
    }
}

impl Llm {
    /// The hotkey: show the window (with the keyboard), or hide it when it has it.
    pub(super) fn chat_toggle(&mut self, cx: &Cx) {
        if self.chat.visible() && (self.chat.ui.key)() {
            (self.chat.ui.hide)();
            self.chat.refocus();
        } else {
            self.summon(None, cx);
        }
    }

    /// Show the window on thread `id` (else the one it showed, the newest, or a new one).
    pub(super) fn summon(&mut self, id: Option<&str>, cx: &Cx) {
        if !self.chat.visible() {
            self.chat.front = (self.chat.ui.front)();
        }
        if let Some(id) = id.filter(|id| self.chat.thread.as_ref().is_none_or(|t| t.id != *id)) {
            self.load(id, cx);
        }
        if self.chat.thread.is_none() {
            let newest = self.settings.history.then(|| cx.store.llm_threads(1).pop()).flatten();
            match newest {
                Some(t) => self.load(&t.id, cx),
                None => self.new_thread(),
            }
        }
        if !self.chat.opened {
            (self.chat.ui.open)();
            self.chat.opened = true;
        }
        self.chat_draw();
        (self.chat.ui.show)();
    }

    /// Show stored thread `id`; a thread no longer stored shows a notice instead.
    fn load(&mut self, id: &str, cx: &Cx) {
        // A reply in flight is stopped and kept in its own thread first.
        self.stop();
        self.pump(cx);
        self.chat.waiting = None;
        match cx.store.llm_thread(id) {
            Some((t, msgs)) => {
                self.chat.thread = Some(Thread { id: t.id, server: t.server, model: t.model, msgs });
                self.chat.notice = None;
            }
            None => self.chat.notice = Some(format!("Chat {id} is no longer kept")),
        }
    }

    /// Start an empty thread on the picked (else default) server and model.
    fn new_thread(&mut self) {
        let (server, model) = if let Some((s, m)) = &self.chat.pick {
            (s.clone(), m.clone())
        } else {
            let server = self.settings.normal(None).map(|s| s.name.clone()).unwrap_or_default();
            (server, self.settings.default_model.clone())
        };
        let id = (self.chat.ui.new_id)();
        self.chat.thread = Some(Thread { id, server, model, msgs: vec![] });
        self.chat.waiting = None;
        self.chat.notice = None;
    }

    /// The `models` view's Enter: the shown thread uses `server` and `model` from its next
    /// prompt on, and so do new threads.
    pub(super) fn pick(&mut self, server: &str, model: &str) {
        self.chat.pick = Some((server.into(), model.into()));
        if let Some(t) = &mut self.chat.thread {
            (t.server, t.model) = (server.into(), model.into());
        }
    }

    /// Handle what the window queued and the reply's new pieces, then redraw if it shows.
    pub(super) fn chat_drain(&mut self, cx: &Cx) {
        for note in (self.chat.ui.take)() {
            match note {
                Note::Submit(text) => self.submit(text, cx),
                Note::Key(Command::New) => {
                    // The reply so far is kept in the thread it belongs to.
                    self.stop();
                    self.pump(cx);
                    self.new_thread();
                }
                Note::Key(Command::Stop) => self.stop(),
                Note::Key(Command::Close) => {
                    (self.chat.ui.hide)();
                    self.chat.refocus();
                }
                Note::Closed => self.chat.refocus(),
            }
        }
        self.pump(cx);
        self.retry_waiting(cx);
        if self.chat.visible() {
            self.chat_draw();
        }
    }

    /// Send `text`, or keep it waiting for the model list, or put it back with a notice.
    fn submit(&mut self, text: String, cx: &Cx) {
        if self.chat.reply.is_some() || self.chat.waiting.is_some() {
            (self.chat.ui.set_input)(&text);
            self.chat.notice = Some("A reply is on its way; ⌘. stops it".into());
            return;
        }
        if self.chat.thread.is_none() {
            self.new_thread();
        }
        self.chat.waiting = Some(text);
        self.retry_waiting(cx);
    }

    /// The server of the shown thread, if it is still a normal one in the config.
    fn server(&self) -> Result<Server, String> {
        let name = self.chat.thread.as_ref().map(|t| t.server.as_str());
        self.settings.normal(name).cloned()
    }

    /// Send the waiting prompt once its thread has a model: the thread's, else the first
    /// one its server lists (fetched now if never).
    fn retry_waiting(&mut self, cx: &Cx) {
        if self.chat.waiting.is_none() {
            return;
        }
        let Some(unresolved) = self.chat.thread.as_ref().map(|t| t.model.is_empty()) else { return };
        let server = match self.server() {
            Ok(s) => s,
            Err(e) => return self.give_back(e),
        };
        if unresolved {
            let listed = self.shared.lock().models.get(&server.name).cloned().unwrap_or_default();
            let first = match listed.result {
                _ if listed.fetching => return,
                None => {
                    io::fetch_models(&self.shared, &server, self.hooks);
                    return;
                }
                Some(Err(e)) => return self.give_back(format!("llm: {}: {e}", server.name)),
                Some(Ok(list)) => list.into_iter().next(),
            };
            let Some(first) = first else { return self.give_back(format!("llm: {} lists no models", server.name)) };
            if let Some(t) = &mut self.chat.thread {
                t.model = first.id;
            }
        }
        if let Some(text) = self.chat.waiting.take() {
            self.send(text, &server, cx);
        }
    }

    /// The waiting prompt goes back into the input, with why.
    fn give_back(&mut self, why: String) {
        if let Some(text) = self.chat.waiting.take() {
            (self.chat.ui.set_input)(&text);
        }
        self.chat.notice = Some(why);
    }

    fn send(&mut self, text: String, server: &Server, cx: &Cx) {
        let now = (self.chat.ui.now)();
        let Some(t) = &mut self.chat.thread else { return };
        t.server.clone_from(&server.name);
        t.msgs.push(Msg { who: Who::User, model: t.model.clone(), body: text, end: End::Done, ts: now });
        let turns: Vec<Turn> = t
            .msgs
            .iter()
            .filter(|m| !m.body.is_empty())
            .map(|m| Turn { role: if m.who == Who::User { Role::User } else { Role::Assistant }, content: &m.body })
            .collect();
        let s = &self.settings;
        let body = openai::chat_body(&openai::Chat { model: &t.model, system: &s.system_prompt, turns: &turns, max_tokens: s.max_tokens });
        let stream = io::chat(&self.shared, server, body, s.timeout_secs, self.hooks);
        let live = Live { model: t.model.clone(), ..Live::default() };
        self.chat.reply = Some(Reply { stream, live });
        self.chat.notice = None;
        self.save_last(cx);
    }

    /// Stop the reply in flight; what arrived is kept on the next pump.
    fn stop(&mut self) {
        if let Some(r) = &self.chat.reply {
            io::cancel(&self.shared, r.stream);
        }
    }

    /// Take the reply's new pieces; once it ended, keep it as a message.
    fn pump(&mut self, cx: &Cx) {
        let Some(r) = &mut self.chat.reply else { return };
        let Some((pieces, status)) = self.shared.take(r.stream) else {
            // Gone (the module stopped every call): keep what arrived as failed.
            return self.end_reply(End::Failed("the request was dropped".into()), cx);
        };
        for p in pieces {
            match p {
                Piece::Text(t) => r.live.text.push_str(&t),
                Piece::Reasoning(_) => r.live.thinking = true,
                _ => {}
            }
        }
        match status {
            Status::Running => {}
            Status::Done => self.end_reply(End::Done, cx),
            Status::Cancelled => self.end_reply(End::Stopped, cx),
            Status::Failed(e) => self.end_reply(End::Failed(e), cx),
        }
    }

    fn end_reply(&mut self, end: End, cx: &Cx) {
        let (Some(r), Some(t)) = (self.chat.reply.take(), &mut self.chat.thread) else { return };
        let ts = (self.chat.ui.now)();
        t.msgs.push(Msg { who: Who::Assistant, model: r.live.model, body: r.live.text, end, ts });
        self.save_last(cx);
    }

    /// Write the thread's last message (with `history`).
    fn save_last(&mut self, cx: &Cx) {
        let Some(t) = self.chat.thread.as_ref().filter(|_| self.settings.history) else { return };
        let Some(msg) = t.msgs.last() else { return };
        let first = t.msgs.first().map_or("", |m| m.body.as_str());
        let title = view::title(first);
        let save = Save { id: &t.id, server: &t.server, title: &title, seq: t.msgs.len() - 1, msg, keep: self.settings.max_threads };
        if let Err(e) = cx.store.llm_save(&save) {
            self.chat.notice = Some(e);
        }
    }

    /// Draw the shown thread: header, rows, notice.
    fn chat_draw(&self) {
        let Some(t) = &self.chat.thread else { return };
        let live = self.chat.reply.as_ref().map(|r| &r.live);
        let bubbles = view::transcript(&t.msgs, self.chat.waiting.as_deref(), live);
        (self.chat.ui.rows)(&view::rows(&bubbles));
        let first = t.msgs.first().map(|m| m.body.as_str()).or(self.chat.waiting.as_deref()).unwrap_or("");
        let phase = self.chat.phase();
        let model = live.map_or(t.model.as_str(), |l| l.model.as_str());
        (self.chat.ui.header)(&view::title(first), &view::subtitle(&t.server, model, phase), view::status(&t.msgs, phase));
        (self.chat.ui.notice)(self.chat.notice.as_deref());
    }
}

#[cfg(test)]
mod tests;
