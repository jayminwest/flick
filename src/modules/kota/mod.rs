//! Module `kota`: KOTA's presence in the menu bar, its pending-card badge and quick ask
//! (plan flick-4354). KOTA is the always-on Claude Code session in a herdr pane on
//! mbp-server. Steps so far: the presence model and `kota status` (flick-3b9b), the
//! poller and `kota refresh` (flick-4f39), the pending-card count (flick-316c), quick
//! ask (flick-039d) and the menu bar item (flick-78b9, `item.rs`).
//!
//! Presence: a round runs `herdr [--machine <machine>] agent list` and curl on kota-dash's
//! `/ok` in parallel (`io.rs`), finds the KOTA pane (agent `claude` in `cwd`) and maps
//! the round to thinking, blocked, idle, degraded, down, offline or unknown, debounced
//! (`presence.rs`). `view.rs` renders it as plain data.
//!
//! Rounds run on a timer only when the `[kota]` table sets a key (any key; an empty
//! `[kota]` counts as none, `Section::is_set`): every `poll_secs` (default 60), every 15 s
//! while KOTA works, and 15 s after a first failure. Without one, an idle Flick runs no
//! thread and no child for this module: `[kota]` is in the same config.toml on every Mac,
//! and only the Mac that wants KOTA's presence should poll. `kota refresh` runs a round on
//! demand either way (at most one per 10 s). Sleep and lock stop the timer; wake and
//! unlock run a round after 5 s.
//!
//! Quick ask (`ask.rs`): `flick kota ask <text>` and the launcher view `kota/ask` (the
//! `hotkey`; the search field holds the question, Enter sends it) post a pending KOTA card
//! and send the text to KOTA's `kota-ask` over ssh, on a thread. The verb answers once ssh
//! is done (`core::later`): exit 1 with the reason when it failed. An ask polls fast for
//! `fast_secs`, and so does the view while it shows. The view lists the last asks (memory
//! only: the message history is the `message` module's). Its Open Chat row (first while
//! the field is empty) is the `message` module's item `message:chat:open`: the KOTA chat
//! window that `[message] chat_hotkey` toggles (flick-ed63).
//!
//! Menu bar item (`item.rs`): the presence glyph plus the count of cards waiting on the
//! user and unread posts (`Event::CardsPending`), shown while rounds run on a timer and `status_item` is
//! true. Its menu has the state rows, Ask KOTA… (hotkey `ask`), Open Chat (the `message`
//! module's hotkey `chat:open`), Inbox (hotkey `inbox`: the `message` module's `recent`
//! view), Open Dashboard and Refresh Now; opening it
//! starts a round. A change to down notifies when `notify_down` is true.
//!
//! `flick kota status [--json]` (no I/O) and `flick kota refresh` are allowed over the
//! network: both only read, and refresh is rate-limited. `kota ask` is not: a peer (KOTA
//! included) must not make this Mac ssh text to KOTA.
//!
//! Table `[kota]` (`settings.rs`): `machine`, `cwd`, `pane`, `herdr`, `dash`, `ssh`,
//! `kota_ask`, `poll_secs`, `fast_secs`, `status_item`, `notify_down`, `hotkey`.

mod ask;
mod io;
mod item;
mod presence;
mod run;
mod settings;
#[cfg(test)]
mod testkit;
mod view;
mod wire;

use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Section;
use crate::core::{Binding, Cx, Event, Icon, Item, ItemId, ListView, Module, Outcome, later, unknown_verb};
use ask::{Ask, Status};
use io::{Hooks, Shared};
use item::{CHAT, INBOX, INBOX_VIEW, Shown, Ui};
use settings::Settings;
use view::{Badge, Extra, Polling};

pub const ID: &str = "kota";
/// The ask view's name, its item key and the hotkey's key.
const ASK: &str = "ask";
/// While the ask view shows, each refresh keeps fast polling on this long.
const VIEW_FAST_SECS: u64 = 2 * presence::FAST_SECS;

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub struct Kota {
    settings: Settings,
    /// The `[kota]` table sets a key: rounds run on a timer.
    active: bool,
    /// `Started` arrived: threads may run. Before it (and in a throwaway instance that a
    /// reload only configures) nothing starts.
    started: bool,
    shared: Arc<Shared>,
    hooks: Hooks,
    /// Cards waiting on the user and unread posts, from the last `Event::CardsPending` (the
    /// `message` module owns them).
    badge: Badge,
    ui: Ui,
    /// What the menu bar item shows; `None` while it is hidden.
    shown: Option<Shown>,
}

impl Default for Kota {
    fn default() -> Self {
        Kota::with_hooks(wire::HOOKS, wire::UI)
    }
}

impl Drop for Kota {
    fn drop(&mut self) {
        self.shared.stop();
        if self.shown.is_some() {
            (self.ui.hide)();
        }
    }
}

impl Kota {
    fn with_hooks(hooks: Hooks, ui: Ui) -> Kota {
        let shared = Arc::default();
        Kota { settings: Settings::default(), active: false, started: false, shared, hooks, badge: Badge::default(), ui, shown: None }
    }

    /// Timed rounds run.
    fn polling(&self) -> bool {
        self.started && self.active
    }

    fn status(&self, json: bool) -> String {
        let polling = match (self.active, self.settings.poll_secs) {
            (false, _) => Polling::Off,
            (true, 0) => Polling::OnDemand,
            (true, secs) => Polling::Every(secs),
        };
        let extra = Extra { badge: self.badge, polling };
        let p = self.shared.lock();
        if json {
            return view::status_json(&p.presence, extra).to_string();
        }
        let now = (self.hooks.now)();
        let offset = (self.hooks.utc_offset)(i64::try_from(now).unwrap_or(0));
        view::status_text(&p.presence, extra, now, offset)
    }

    /// Poll fast until `until` (unix seconds) and let a round start if that makes one due.
    /// `kick`: with `poll_secs` 0 (on demand), start one now (rate-limited).
    fn poll_fast(&self, until: u64, kick: bool) {
        {
            let mut p = self.shared.lock();
            p.fast_until = Some(p.fast_until.unwrap_or(0).max(until));
        }
        if !self.polling() {
            return;
        }
        let (shared, s, hooks) = (&self.shared, &self.settings, self.hooks);
        if s.poll_secs > 0 {
            io::tick(shared, s, hooks);
        } else if kick {
            drop(io::refresh(shared, s, hooks));
        }
    }

    /// Ask KOTA `text` on a thread (`ask::send`). `cli`: the control request answers with
    /// the outcome once ssh is done (`core::later`).
    fn ask(&self, text: &str, cli: bool) -> Result<String, String> {
        let text = ask::check(text)?;
        let id = ask::new_id();
        let now = (self.hooks.now)();
        let ask = Ask { id: id.clone(), text: text.clone(), at: now, status: Status::Sending };
        ask::remember(&mut self.shared.lock().asks, ask);
        self.poll_fast(now.saturating_add(self.settings.fast_secs), true);
        let answer = cli.then(later::answer_later);
        let (shared, s, hooks, req) = (Arc::clone(&self.shared), self.settings.clone(), self.hooks, id.clone());
        let spawned = thread::Builder::new().name("kota-ask".into()).spawn(move || {
            let result = ask::send(&req, &text, &s, hooks);
            ask::settle(&mut shared.lock().asks, &req, &result);
            (hooks.post)();
            if let Some(tx) = answer {
                let _ = tx.send(result.map(|a| format!("Asked KOTA ({req}): {a}")).map_err(|e| format!("kota ask: {e}")));
            }
        });
        if let Err(e) = spawned {
            // The ask stays "sending…" in the view; this is the answer.
            return Err(format!("kota ask: no thread: {e}"));
        }
        Ok(format!("Asking KOTA ({id})"))
    }

    /// The ask view, polling fast while it shows.
    fn ask_view(&self) -> ListView {
        self.poll_fast((self.hooks.now)().saturating_add(VIEW_FAST_SECS), true);
        ListView {
            placeholder: "Ask KOTA…".into(),
            footer: "KOTA  ·  ↵ sends".into(),
            empty: "Type a question, then ↵ to send it to KOTA".into(),
            escape_hides: true,
            ..ListView::new(ID, ASK)
        }
    }
}

impl Module for Kota {
    fn id(&self) -> &'static str {
        ID
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let settings = table.get::<Settings>()?.check()?;
        let active = table.is_set();
        let old = std::mem::replace(&mut self.settings, settings);
        let was = std::mem::replace(&mut self.active, active);
        if !self.started {
            return Ok(());
        }
        let s = &self.settings;
        let target = |s: &Settings| (s.machine.clone(), s.cwd.clone(), s.pane.clone(), s.herdr.clone(), s.dash.clone());
        if target(&old) != target(s) {
            self.shared.reset();
        } else if was && !active {
            self.shared.stop();
        }
        if self.active && (old != *s || !was) {
            io::tick(&self.shared, s, self.hooks);
        }
        self.sync_item();
        Ok(())
    }

    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        (view == ASK).then(|| self.ask_view())
    }

    /// The ask view: the question as one item, KOTA's state in the footer, the last asks
    /// as text.
    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        if view.name != ASK {
            return;
        }
        let now = (self.hooks.now)();
        self.poll_fast(now.saturating_add(VIEW_FAST_SECS), false);
        let q = cx.query.trim();
        let ask = (!q.is_empty()).then(|| Item {
            subtitle: "KOTA's reply replaces the pending card".into(),
            ..Item::new(ItemId::new(ID, ASK).with_arg(q), format!("Ask KOTA: {q}"), "Send", Icon::Symbol("paperplane"))
        });
        // The `message` module's item: Enter goes to it (`Registry::activate` routes by id).
        let chat = Item {
            subtitle: "The KOTA chat window, on its last thread".into(),
            ..Item::new(ItemId::new(CHAT.0, CHAT.1), "Open Chat", "Open", Icon::Symbol("bubble.left.and.bubble.right"))
        };
        let verb = if ask.is_some() { "sends" } else { "opens chat" };
        view.items = ask.into_iter().chain([chat]).collect();
        let p = self.shared.lock();
        view.footer = format!("{}  ·  ↵ {verb}", view::headline(&p.presence, now));
        view.text = view::asks_text(&p.asks, (self.hooks.utc_offset)(i64::try_from(now).unwrap_or(0)));
    }

    fn activate(&mut self, id: &ItemId, _cx: &mut Cx) -> Outcome {
        match (id.key(), id.arg()) {
            (ASK, Some(text)) => match self.ask(text, false) {
                Ok(_) => Outcome::Hide,
                Err(e) => Outcome::Stay(Some(e)),
            },
            _ => Outcome::Stay(None),
        }
    }

    fn hotkeys(&self) -> Vec<Binding> {
        let spec = self.settings.hotkey.as_ref().filter(|s| !s.trim().is_empty());
        spec.map(|s| Binding { spec: s.clone(), key: Ok(ASK.into()) }).into_iter().collect()
    }

    /// `ask` (the `hotkey`, and the menu's Ask KOTA…) opens the ask view; `inbox` (the
    /// menu's Inbox) asks for the cards view by name.
    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        match key {
            ASK => Some(self.ask_view()),
            INBOX => Some(ListView::new(INBOX_VIEW.0, INBOX_VIEW.1)),
            _ => None,
        }
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        match event {
            Event::Started => self.started = true,
            Event::CardsPending { count, unread } => self.badge = Badge { pending: count, unread },
            Event::ModuleChanged { module: ID } => self.requests(),
            _ => {}
        }
        if !self.polling() {
            return false;
        }
        let (shared, s, hooks) = (&self.shared, &self.settings, self.hooks);
        match event {
            Event::Started | Event::ModuleChanged { module: ID } => io::tick(shared, s, hooks),
            Event::Sleep | Event::Locked => io::suspend(shared),
            Event::Wake | Event::Unlocked => io::resume(shared, s, hooks),
            Event::LauncherOpened if s.poll_secs == 0 => drop(io::refresh(shared, s, hooks)),
            _ => {}
        }
        if matches!(
            event,
            Event::Started
                | Event::ModuleChanged { module: ID }
                | Event::CardsPending { .. }
                | Event::Sleep
                | Event::Locked
                | Event::Wake
                | Event::Unlocked
                | Event::LauncherOpened
        ) {
            self.sync_item();
        }
        false
    }

    /// `--json` (`cx.json`) makes `status` answer with JSON (`view::status_json`). `ask`
    /// answers later, once ssh is done.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v] if v == "status" => Ok(self.status(cx.json)),
            [v] if v == "refresh" => Ok(io::refresh(&self.shared, &self.settings, self.hooks)),
            [v, text @ ..] if v == ASK => self.ask(&text.join(" "), true),
            _ => Err(unknown_verb(ID, args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "kota status | kota refresh | kota ask <text>"
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_ask;
