//! The private chat window (flick-c325): the root item `llm:private` and `[llm]
//! private_hotkey` open a private surface ("llm-private") on a `PrivateSession`. Nothing of
//! it is written anywhere: these functions get no `Cx`, so no store; the module has no verb
//! for it; it posts only the text-free `ModuleChanged`; it never notifies. The real hooks are
//! `wire::PRIVATE_UI`; tests use `testkit::PRIVATE`, so no test opens a window.
//!
//! - Server gate: only a `private = true` server (`Settings::private`). With none, the item
//!   says `settings::NO_PRIVATE` and the hotkey shows it in the launcher; it never falls back
//!   to a normal server.
//! - Window: `surface::Spec::private` with `BANNER` (out of screenshots and screen sharing,
//!   text services off, fixed title `TITLE`). The header never shows chat text.
//! - Model: `default_model` if the private server lists it, else the first it lists (the list
//!   is fetched when the session opens; a prompt sent before it lands waits for it).
//! - Keys: Return sends, ⌘. stops (what arrived stays), ⌘N clears, ⌘C with nothing selected
//!   copies the last reply through `pasteboard::set_text_concealed` (clip history skips it),
//!   ⌘W or Esc hides and clears. The hotkey hides (and clears) it when it has the keyboard.
//! - Wiped (`Wipe`): window hidden, `Event::Locked`, `Event::Sleep`, config reload (the
//!   running module only, `configure`), quit (`wire`'s `app::on_terminate` hook calls
//!   `wipe_for_quit` through weak handles; it borrows no app state). A wipe drops the session
//!   (so the next prompt opens a new one through the gate), stops its stream and wipes what
//!   arrived, empties the window's rows and input, and says why in the notice.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use super::Llm;
use super::chat::{self, Command, Note};
use super::io::{self, Hooks, Shared, Status};
use super::openai::{self, Role};
use super::private::{Entry, PrivateSession, Wipe};
use super::settings::Settings;
use super::transport::wipe_string;
use super::view::THINKING;
use crate::platform::surface::rows::{BubbleState, Side};
use crate::platform::surface::{self, Key, Keystroke};

/// The window title and the header title: fixed, never chat text.
pub const TITLE: &str = "Private Chat";
/// The private surface's banner.
pub const BANNER: &str = "Private - nothing is saved";
/// A reply with no text yet and no reasoning: the server may be loading the model.
pub const WAITING: &str = "Waiting for the model (a cold start loads it first)…";
pub const COPIED: &str = "Copied the last reply (left out of clipboard history)";
pub const NOTHING_TO_COPY: &str = "No reply to copy";
const BUSY: &str = "A reply is on its way; ⌘. stops it";

/// What the private window needs from the system: the window (`chat::Ui`, on its own
/// surface and note queue), the concealed copy, and the quit hook.
#[derive(Clone, Copy)]
pub struct Ui {
    pub win: chat::Ui,
    /// `pasteboard::set_text_concealed`.
    pub copy: fn(&str),
    /// Make quit wipe the room (`wipe_for_quit`); called each time the window opens.
    pub hook_quit: fn(&Arc<Room>, &Arc<Shared>),
}

/// The private chat while it has a session: the transcript and a prompt waiting for the
/// model list (wiped on drop).
pub struct Open {
    pub session: PrivateSession,
    waiting: Option<String>,
}

impl Drop for Open {
    fn drop(&mut self) {
        if let Some(w) = &mut self.waiting {
            wipe_string(w);
        }
    }
}

/// Where the session lives: shared with the quit hook, which may not borrow the module.
pub type Room = Mutex<Option<Open>>;

fn lock(room: &Room) -> MutexGuard<'_, Option<Open>> {
    room.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The quit hook's work: drop the session (wiping it) and stop every call (wiping what
/// arrived). Skips a room that is locked right now; the process is ending anyway.
pub fn wipe_for_quit(room: &Weak<Room>, shared: &Weak<Shared>) {
    if let Some(room) = room.upgrade()
        && let Ok(mut r) = room.try_lock()
    {
        drop(r.take());
    }
    if let Some(shared) = shared.upgrade() {
        shared.stop();
    }
}

/// The private chat's own keys: the chat's, and ⌘C (reaches the module only with nothing
/// selected).
pub fn binding(k: Keystroke) -> Option<Command> {
    let copy = k.cmd && !k.shift && !k.opt && k.key == Key::Char('c');
    if copy { Some(Command::Copy) } else { chat::binding(k) }
}

/// The private window's state in the module.
pub struct Private {
    pub ui: Ui,
    pub room: Arc<Room>,
    opened: bool,
    front: Option<i32>,
    notice: Option<String>,
}

impl Private {
    pub fn new(ui: Ui) -> Private {
        Private { ui, room: Arc::default(), opened: false, front: None, notice: None }
    }

    fn visible(&self) -> bool {
        self.opened && (self.ui.win.visible)()
    }

    fn refocus(&mut self) {
        if let Some(pid) = self.front.take()
            && (self.ui.win.front)() == Some(pid)
        {
            (self.ui.win.refocus)(pid);
        }
    }
}

/// The room with a session in it, opening one on the private server (and fetching its model
/// list) if it has none. `Err`: no private server.
fn enter<'a>(room: &'a Room, settings: &Settings, shared: &Arc<Shared>, hooks: Hooks) -> Result<MutexGuard<'a, Option<Open>>, String> {
    let mut r = lock(room);
    if r.is_none() {
        let session = PrivateSession::open(settings, None)?;
        io::fetch_models(shared, session.server(), hooks);
        *r = Some(Open { session, waiting: None });
    }
    Ok(r)
}

/// One bubble: its text borrowed from the transcript, never copied.
struct Shown<'a> {
    key: String,
    side: Side,
    header: String,
    md: &'a str,
    state: BubbleState,
    version: u64,
}

fn shown(key: String, side: Side, header: String, md: &str, state: BubbleState) -> Shown<'_> {
    let mut h = DefaultHasher::new();
    (&header, md, format!("{side:?}{state:?}")).hash(&mut h);
    Shown { key, side, header, md, state, version: h.finish() }
}

fn entry<'a>(i: usize, e: &'a Entry, model: &str) -> Shown<'a> {
    let key = format!("p{i}");
    if e.role == Role::User {
        return shown(key, Side::Mine, String::new(), &e.text, BubbleState::Done);
    }
    let text = e.text.as_str();
    let (header, md, state) = match &e.status {
        Status::Running if text.is_empty() => (model.into(), if e.reasoning.is_empty() { WAITING } else { THINKING }, BubbleState::Pending),
        Status::Running => (model.into(), text, BubbleState::Streaming),
        Status::Done => (model.into(), text, BubbleState::Done),
        Status::Cancelled => (format!("{model} · stopped"), text, BubbleState::Done),
        Status::Failed(err) => (format!("{model} · failed"), if text.is_empty() { err.as_str() } else { text }, BubbleState::Failed),
    };
    shown(key, Side::Theirs, header, md, state)
}

/// The header's subtitle: the server and model (never chat text) and the keys that apply.
pub fn subtitle(open: Option<&Open>) -> String {
    let Some(o) = open else { return "Nothing is kept  ·  type to start a new private chat".into() };
    let server = o.session.server().name.as_str();
    match (o.session.model(), o.session.streaming()) {
        (model, Some(_)) => format!("{server} · {model}  ·  ⌘. stops"),
        ("", None) => format!("{server} · loading models…"),
        (model, None) => format!("{server} · {model}  ·  ⌘C copies the reply  ·  ⌘N or Esc clears"),
    }
}

fn status(open: Option<&Open>) -> surface::Status {
    let Some(o) = open else { return surface::Status::Idle };
    if o.session.streaming().is_some() || o.waiting.is_some() {
        return surface::Status::Busy;
    }
    match o.session.entries().last() {
        Some(Entry { status: Status::Failed(_), .. }) => surface::Status::Error,
        _ => surface::Status::Idle,
    }
}

impl Llm {
    /// The hotkey: show the private window, or hide and clear it when it has the keyboard.
    /// `Err` (`settings::NO_PRIVATE`): no private server.
    pub(super) fn private_toggle(&mut self) -> Result<(), String> {
        if self.private.visible() && (self.private.ui.win.key)() {
            self.private_close();
            return Ok(());
        }
        self.private_summon()
    }

    /// Show the private window with the keyboard, on a session through the server gate.
    pub(super) fn private_summon(&mut self) -> Result<(), String> {
        drop(enter(&self.private.room, &self.settings, &self.shared, self.hooks)?);
        if !self.private.visible() {
            self.private.front = (self.private.ui.win.front)();
        }
        if !self.private.opened {
            (self.private.ui.win.open)();
            self.private.opened = true;
        }
        (self.private.ui.hook_quit)(&self.private.room, &self.shared);
        self.private_draw();
        (self.private.ui.win.show)();
        Ok(())
    }

    fn private_close(&mut self) {
        (self.private.ui.win.hide)();
        self.private.refocus();
        self.private_wipe(Wipe::Closed);
    }

    /// Handle what the private window queued and its reply's new pieces, then redraw.
    pub(super) fn private_drain(&mut self) {
        for note in (self.private.ui.win.take)() {
            match note {
                Note::Submit(text) => self.private_submit(text),
                Note::Key(Command::New) => self.private_wipe(Wipe::Cleared),
                Note::Key(Command::Stop) => self.private_stop(),
                Note::Key(Command::Copy) => self.private_copy(),
                Note::Key(Command::Close) => self.private_close(),
                Note::Closed => {
                    self.private.refocus();
                    self.private_wipe(Wipe::Closed);
                }
            }
        }
        self.private_pump();
        self.private_resolve();
        if self.private.visible() {
            self.private_draw();
        }
    }

    /// Hold `text` for sending, or put it back (and wipe it) with why.
    fn private_submit(&mut self, mut text: String) {
        let mut room = match enter(&self.private.room, &self.settings, &self.shared, self.hooks) {
            Ok(r) => r,
            Err(e) => {
                wipe_string(&mut text);
                self.private.notice = Some(e);
                return;
            }
        };
        let Some(open) = room.as_mut() else { return };
        if open.session.streaming().is_some() || open.waiting.is_some() {
            (self.private.ui.win.set_input)(&text);
            wipe_string(&mut text);
            self.private.notice = Some(BUSY.into());
            return;
        }
        open.waiting = Some(text);
        drop(room);
        self.private_resolve();
    }

    /// Pick the model once the private server's list landed, then send the waiting prompt.
    fn private_resolve(&mut self) {
        let mut room = lock(&self.private.room);
        let Some(open) = room.as_mut() else { return };
        if open.session.model().is_empty() {
            let name = open.session.server().name.clone();
            let listed = self.shared.lock().models.get(&name).cloned().unwrap_or_default();
            let why = match listed.result {
                _ if listed.fetching => return,
                None => {
                    io::fetch_models(&self.shared, open.session.server(), self.hooks);
                    return;
                }
                Some(Err(e)) => format!("llm: {name}: {e}"),
                Some(Ok(list)) => match list.iter().find(|m| m.id == self.settings.default_model).or(list.first()) {
                    Some(m) => {
                        open.session.set_model(&m.id);
                        String::new()
                    }
                    None => format!("llm: {name} lists no models"),
                },
            };
            if !why.is_empty() {
                // Put the prompt back and ask again, so the next one may find the list.
                if let Some(mut w) = open.waiting.take() {
                    (self.private.ui.win.set_input)(&w);
                    wipe_string(&mut w);
                    self.private.notice = Some(why);
                    io::fetch_models(&self.shared, open.session.server(), self.hooks);
                }
                return;
            }
        }
        let Some(text) = open.waiting.take() else { return };
        let s = &self.settings;
        match open.session.send(text, &s.system_prompt, s.max_tokens) {
            Ok(body) => {
                let id = io::chat(&self.shared, open.session.server(), body, s.timeout_secs, self.hooks);
                open.session.started(id);
                self.private.notice = None;
            }
            Err(e) => self.private.notice = Some(e),
        }
    }

    /// Move the stream's new pieces into the transcript (`absorb` wipes them).
    fn private_pump(&mut self) {
        let mut room = lock(&self.private.room);
        let Some(open) = room.as_mut() else { return };
        let Some(id) = open.session.streaming() else { return };
        let (pieces, status) = self.shared.take(id).unwrap_or_else(|| (vec![], Status::Failed("the request was dropped".into())));
        if let Status::Failed(e) = &status {
            self.private.notice = Some(format!("llm: {e}"));
        }
        open.session.absorb(id, pieces, status);
    }

    fn private_stop(&mut self) {
        if let Some(id) = lock(&self.private.room).as_ref().and_then(|o| o.session.streaming()) {
            io::cancel(&self.shared, id);
        }
    }

    /// ⌘C: the last reply to the pasteboard, marked concealed and transient.
    fn private_copy(&mut self) {
        let room = lock(&self.private.room);
        let entries = room.as_ref().map_or(&[][..], |o| o.session.entries());
        let last = entries.iter().rev().find(|e| e.role == Role::Assistant && !e.text.is_empty());
        let notice = match last {
            Some(e) => {
                (self.private.ui.copy)(&e.text);
                COPIED
            }
            None => NOTHING_TO_COPY,
        };
        self.private.notice = Some(notice.into());
    }

    /// Wipe the private chat for `why`: the session, its stream and what arrived, and the
    /// window's rows and input. No window calls if it never opened.
    pub(super) fn private_wipe(&mut self, why: Wipe) {
        let gone = lock(&self.private.room).take();
        if let Some(mut open) = gone
            && let Some(id) = open.session.clear()
        {
            io::cancel(&self.shared, id);
            if let Some((mut pieces, _)) = self.shared.take(id) {
                openai::wipe_pieces(&mut pieces);
            }
        }
        if !self.private.opened {
            return;
        }
        (self.private.ui.win.set_input)("");
        self.private.notice = Some(why.notice().into());
        self.private_draw();
    }

    fn private_draw(&self) {
        let ui = self.private.ui.win;
        let room = lock(&self.private.room);
        let open = room.as_ref();
        let mut bubbles = vec![];
        if let Some(o) = open {
            let model = o.session.model();
            bubbles.extend(o.session.entries().iter().enumerate().map(|(i, e)| entry(i, e, model)));
            if let Some(w) = &o.waiting {
                bubbles.push(shown("waiting".into(), Side::Mine, String::new(), w, BubbleState::Pending));
            }
        }
        let rows: Vec<surface::Row> = bubbles
            .iter()
            .map(|b| surface::Row::Bubble { key: &b.key, version: b.version, side: b.side, header: &b.header, time: "", md: b.md, state: b.state })
            .collect();
        (ui.rows)(&rows);
        (ui.header)(TITLE, &subtitle(open), status(open));
        (ui.notice)(self.private.notice.as_deref());
    }
}

#[cfg(test)]
mod tests;
