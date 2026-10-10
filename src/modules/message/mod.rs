//! Module `message`: short messages posted to this Mac, usually by an agent on another one
//! (`flick --host <mac> message post ...`), shown in a small corner panel that does not take
//! focus (`platform::hud`), as a notification, or both, and kept in a history list. A post
//! may be `--pending` (a placeholder, e.g. "sent to KOTA") that a later post with
//! `--reply-to <its id>` replaces. Ids are `message:list` (root item, view `recent`) and
//! `message:<message id>` (rows of `recent`; Enter copies the body, ⌘K Show / Open Link /
//! Copy). Table `[message]`: `name`, `style`, `position`, `width`, `timeout_secs`,
//! `max_cards`, `max_history`, `chat_history`, `chat_threads`, `sound`, `hotkey` (opens `recent`), `card_hotkey` (moves the
//! keyboard into the newest card, or gives it back), `action_command`,
//! `pending_timeout_secs`, `card_timeout_secs`, `chat_hotkey` (shows or hides the chat
//! window), `kota_host`, `kota_ask` (where a chat question goes). Table `messages` holds the history. Each
//! message shows as its own card, keyed by its id. `message card <verb>` posts and manages
//! structured cards (`card.rs`, `core::card`), stored in the same table and drawn by the HUD's
//! card renderer; presses and dismissals are handled in `dispatch.rs`, KOTA sends in `run.rs`,
//! local actions that run a process (`script`, `flick`, `shell`) in `local.rs`. Chat threads
//! (`post --thread`, `--partial` streaming) are stored per thread; `thread.rs` reads them;
//! `chat/` is the KOTA chat window (`message chat`, `message ask`, view `threads`, ids
//! `message:thread:<t>`). While the window shows a thread, posts to it show no card and
//! play no sound (`chat::model::alert`).

mod card;
mod chat;
mod dispatch;
mod local;
mod pending;
mod run;
mod settings;
pub mod store;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_card;
#[cfg(test)]
mod tests_local;
#[cfg(test)]
mod tests_pending;
#[cfg(test)]
mod tests_press;
#[cfg(test)]
mod tests_thread;
mod text;
mod thread;
mod wire;

use std::collections::HashSet;

use serde::Serialize;

use crate::config::Section;
use crate::core::card::State;
use crate::core::{Action, Binding, Cx, Event, Icon, Item, ItemId, ListView, Module, Outcome, unknown_verb};
use crate::platform::hud::{Content, Options, Placement, TextCard};
use settings::{Settings, Style};
use store::{Keep, Message, Messages, Progress};
use wire::Env;

const LIST: &str = "list";
const RECENT: &str = "recent";
/// The hotkey key of `card_hotkey`.
const CARD: &str = "card";
/// The hotkey key of `chat_hotkey`.
const CHAT: &str = "chat";

#[derive(Default)]
pub struct Inbox {
    env: Env,
    settings: Settings,
    /// Cards dismissed (`card dismiss`, the x, Esc, a timeout); a `done` update of one stays
    /// in history.
    dismissed: HashSet<String>,
    /// Dismissed cards that only timed out: they still wait on the user (`pending.rs`).
    expired: HashSet<String>,
    /// The last count of cards waiting on the user sent as `Event::CardsPending`.
    announced: Option<u32>,
    /// Press state by card id (`dispatch.rs`).
    ui: dispatch::Uis,
    /// Presses sent so far; numbers each send.
    presses: u64,
    /// KOTA sends in flight.
    worker: run::Worker<run::Done>,
    /// Local runs in flight (`local.rs`).
    local: run::Worker<local::Done>,
    /// The chat window (`chat/session.rs`).
    chat: chat::session::Chat,
}

/// The JSON answer of `post`.
#[derive(Serialize)]
struct Posted<'a> {
    id: &'a str,
    replaced: bool,
}

impl Inbox {
    fn placement(&self) -> Placement {
        Placement {
            corner: self.settings.position.corner(),
            width: self.settings.width,
            max_cards: self.settings.max_cards,
        }
    }

    /// A message card: timed out like any card, no sound for a placeholder or a reply still
    /// streaming.
    fn options(&self, m: &Message) -> Options {
        Options {
            timeout_secs: self.settings.timeout_secs as f64,
            sound: self.settings.sound && !quiet(m),
            sticky: false,
        }
    }

    fn header<'a>(&'a self, m: &'a Message) -> &'a str {
        m.title.as_deref().unwrap_or(&self.settings.name)
    }

    /// Show `m` the configured way; `panel_only` skips the notification (a re-show). A card
    /// is drawn by the HUD's card renderer with its press state; its notification is its
    /// title and the rest of `plain(card)`.
    fn display(&self, m: &Message, panel_only: bool) {
        (self.env.subscribe)();
        let panel = matches!(self.settings.style, Style::Panel | Style::Both) || panel_only;
        let (header, body, pending) = if let Some(c) = card::stored(m) {
            if panel {
                self.show_card(&c);
            }
            (c.title.clone(), card::body(&c), c.state == State::Pending)
        } else {
            let body = text::plain(&m.body);
            if panel {
                self.show_text(m, &body);
            }
            (self.header(m).to_string(), body, quiet(m))
        };
        if matches!(self.settings.style, Style::Notification | Style::Both) && !panel_only && !pending {
            (self.env.notify)(&m.id, &header, &body);
        }
    }

    fn show_text(&self, m: &Message, body: &str) {
        let context = m.context.as_deref().map(|c| format!("Re: {}", text::preview(c, 80)));
        let time = text::stamp(m.ts, (self.env.now)(), (self.env.utc_offset)(m.ts));
        let card = TextCard {
            header: self.header(m),
            time: &time,
            context: context.as_deref().unwrap_or(""),
            body: &text::clip(body),
            link: m.url.as_deref(),
            pending: m.pending,
        };
        (self.env.show)(&m.id, &Content::Text(card), &self.placement(), &self.options(m));
    }

    /// Save `m`; `took` (its `reply_to` was a pending message, now gone) also removes that
    /// placeholder's card, which `m` takes the place of.
    fn save(&self, m: &Message, took: bool, cx: &Cx) -> Result<(), String> {
        cx.store.put_message(m, self.keep())?;
        if let Some(pending) = m.reply_to.as_deref().filter(|r| took && *r != m.id) {
            (self.env.dismiss)(pending);
        }
        Ok(())
    }

    /// How much history the store keeps.
    fn keep(&self) -> Keep {
        let s = &self.settings;
        Keep { history: s.max_history, per_thread: s.chat_history, threads: s.chat_threads }
    }

    /// Save and show a post; returns its id and whether it replaced a pending message.
    fn post(&self, args: &[String], cx: &Cx) -> Result<(String, bool), String> {
        let post = text::parse_post(args)?;
        let id = post.id.unwrap_or_else(self.env.new_id);
        let first = cx.store.message(&id).is_none();
        let (context, replaced) = quote(post.reply_to.as_deref(), cx);
        let m = Message {
            id: id.clone(),
            ts: (self.env.now)(),
            title: post.title,
            body: post.body,
            url: post.url,
            reply_to: post.reply_to,
            context,
            pending: post.pending,
            thread: post.thread,
            state: if post.partial { Progress::Partial } else { Progress::Done },
            ..Message::default()
        };
        self.save(&m, replaced, cx)?;
        // Its sound is `quiet`'s, as `alert` decides it.
        if chat::model::alert(&m, first, self.chat_showing()).hud {
            self.display(&m, false);
        }
        Ok((id, replaced))
    }

    fn row(&self, m: &Message, now: i64) -> Item {
        let title = match &m.title {
            Some(t) => format!("{t}: {}", text::preview(&text::plain(&m.body), 100)),
            None => text::preview(&text::plain(&m.body), 120),
        };
        let time = text::stamp(m.ts, now, (self.env.utc_offset)(m.ts));
        let context = m.context.as_deref().map(|c| format!("Re: {}", text::preview(c, 60)));
        Item {
            subtitle: [Some(time), context].into_iter().flatten().collect::<Vec<_>>().join("  ·  "),
            accessory: if m.pending { "Pending" } else if m.url.is_some() { "Link" } else { "" }.into(),
            keywords: m.context.iter().cloned().collect(),
            ..Item::new(ItemId::new("message", &m.id), title, "Copy Message", icon(m))
        }
    }

    /// `ls` text: one line per message, `id  time  [title: ]body[ (pending|partial|failed)]`.
    fn ls(&self, list: &[Message], now: i64) -> String {
        let line = |m: &Message| {
            let time = text::stamp(m.ts, now, (self.env.utc_offset)(m.ts));
            let title = m.title.as_ref().map(|t| format!("{t}: ")).unwrap_or_default();
            let mark = match (m.pending, m.state) {
                (true, _) => " (pending)",
                (_, Progress::Partial) => " (partial)",
                (_, Progress::Failed) => " (failed)",
                (_, Progress::Done) => "",
            };
            let body = text::preview(&text::plain(&m.body), 100);
            format!("{}\t{time}\t{title}{body}{mark}", m.id)
        };
        list.iter().map(line).collect::<Vec<_>>().join("\n")
    }

    fn run_verb(&mut self, args: &[String], cx: &Cx) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["card", ..] => self.card_verb(&args[1..], cx),
            ["thread" | "threads", ..] => self.thread_verb(args, cx),
            ["chat" | "ask", ..] => self.chat_verb(args, cx),
            ["post", ..] => {
                let (id, replaced) = self.post(&args[1..], cx)?;
                if cx.json {
                    return serde_json::to_string(&Posted { id: &id, replaced }).map_err(|e| e.to_string());
                }
                Ok(id)
            }
            ["ls", rest @ ..] => {
                let limit = match rest {
                    [] => 20,
                    ["--limit", n] => n.parse().map_err(|_| format!("--limit {n}: not a number"))?,
                    _ => return Err("usage: flick message ls [--limit n]".into()),
                };
                let list = cx.store.messages(limit);
                if cx.json {
                    return serde_json::to_string(&list).map_err(|e| e.to_string());
                }
                Ok(self.ls(&list, (self.env.now)()))
            }
            ["show", rest @ ..] if rest.len() <= 1 => {
                let m = match rest.first() {
                    Some(id) => cx.store.message(id).ok_or(format!("No message {id}"))?,
                    None => cx.store.messages(1).pop().ok_or("No messages")?,
                };
                self.display(&m, true);
                Ok(format!("Showing {}", m.id))
            }
            ["hide"] => {
                (self.env.hide)();
                Ok("Hidden".into())
            }
            _ => Err(unknown_verb("message", args)),
        }
    }
}

/// For a post answering `reply_to`: the body it quotes, and whether it was a pending
/// message, which is taken out of history.
fn quote(reply_to: Option<&str>, cx: &Cx) -> (Option<String>, bool) {
    let Some(r) = reply_to else { return (None, false) };
    match cx.store.take_pending(r) {
        Some(m) => (Some(m.body), true),
        None => (cx.store.message(r).map(|m| m.body), false),
    }
}

/// A placeholder or a reply still streaming: no sound, no notification.
fn quiet(m: &Message) -> bool {
    m.pending || m.state == Progress::Partial
}

fn icon(m: &Message) -> Icon {
    Icon::Symbol(if m.pending { "hourglass" } else { "bubble.left" })
}

fn recent_view(name: &str) -> ListView {
    ListView {
        placeholder: format!("Search {}…", name.to_lowercase()),
        footer: format!("{name}  ·  ↵ copies  ·  ⌘K show or open"),
        empty: "No messages yet".into(),
        ..ListView::new("message", RECENT)
    }
}

impl Module for Inbox {
    fn id(&self) -> &'static str {
        "message"
    }

    fn migrations(&self) -> &'static [&'static str] {
        store::MIGRATIONS
    }

    /// `ModuleChanged` for this module: card presses, dismissals and finished sends. Then,
    /// and at `Started`, the count of cards waiting on the user (`pending.rs`).
    fn on_event(&mut self, event: Event, cx: &mut Cx) -> bool {
        match event {
            Event::ModuleChanged { module: "message" } => {
                self.drain(cx);
                self.chat_drain(cx);
            }
            Event::Started => {}
            _ => return false,
        }
        self.announce(cx);
        false
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let s = table.get::<Settings>()?;
        if s.name.trim().is_empty() {
            return Err("[message]: name is empty".into());
        }
        if !(240.0..=900.0).contains(&s.width) {
            return Err("[message]: width must be 240 to 900".into());
        }
        if s.max_cards == 0 {
            return Err("[message]: max_cards must be at least 1".into());
        }
        for (key, n) in [("max_history", s.max_history), ("chat_history", s.chat_history), ("chat_threads", s.chat_threads)] {
            if n == 0 {
                return Err(format!("[message]: {key} must be at least 1"));
            }
        }
        if s.action_command.first().is_some_and(|p| p.trim().is_empty()) {
            return Err("[message]: action_command needs a program first".into());
        }
        chat::ask::argv(&s.kota_host, &s.kota_ask, "r", "t").map_err(|e| format!("[message]: {e}"))?;
        self.settings = s;
        Ok(())
    }

    fn items(&mut self, cx: &mut Cx) -> Vec<Item> {
        let last = cx.store.messages(1).pop();
        let subtitle = last.map_or_else(
            || "No messages yet".into(),
            |m| text::preview(&text::plain(&m.body), 80),
        );
        vec![Item {
            subtitle,
            accessory: "Command".into(),
            keywords: vec!["messages".into(), "inbox".into(), "replies".into()],
            ..Item::new(ItemId::new("message", LIST), self.settings.name.clone(), "Open", Icon::Symbol("bubble.left.and.bubble.right"))
        }]
    }

    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        match view {
            RECENT => Some(recent_view(&self.settings.name)),
            chat::threads::VIEW => Some(chat::threads::view()),
            _ => None,
        }
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        if view.name == chat::threads::VIEW {
            return self.refresh_threads(view, cx);
        }
        let now = (self.env.now)();
        let list = cx.store.messages(self.settings.max_history);
        let n = list.len();
        let items: Vec<Item> = list.into_iter().map(|m| self.row(&m, now)).collect();
        let order: Vec<String> = items.iter().map(|i| i.id.to_string()).collect();
        // Newest first for equal scores.
        view.items = cx.ranker.rank(cx.query, items, |i| {
            let pos = order.iter().position(|o| *o == i.id.as_str()).unwrap_or(n);
            -(pos as f64) * 1e-3
        });
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        if id.key() == LIST {
            return Outcome::Push(ListView::new("message", RECENT));
        }
        if let Some(t) = chat::threads::thread_of(id.key()) {
            return self.open_thread(t, cx);
        }
        match cx.store.message(id.key()) {
            Some(m) => {
                (self.env.copy)(&text::plain(&m.body));
                Outcome::Stay(Some("Copied message".into()))
            }
            None => Outcome::Stay(None),
        }
    }

    fn actions(&mut self, id: &ItemId, cx: &mut Cx) -> Vec<Action> {
        if id.key() == LIST {
            return vec![chat::threads::action()];
        }
        let Some(m) = (id.key() != LIST).then(|| cx.store.message(id.key())).flatten() else {
            return vec![];
        };
        let mut actions = vec![Action::new("show", "Show in Panel", Icon::Symbol("macwindow"))];
        if m.url.is_some() {
            actions.push(Action::new("open", "Open Link", Icon::Symbol("link")));
        }
        actions.push(Action::new("copy", "Copy Message", Icon::Symbol("doc.on.doc")));
        actions
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        if (id.key(), key) == (LIST, chat::threads::VIEW) {
            return Outcome::Push(chat::threads::view());
        }
        let Some(m) = cx.store.message(id.key()) else { return Outcome::Stay(None) };
        match (key, &m.url) {
            ("show", _) => {
                cx.hide();
                self.display(&m, true);
                Outcome::Hide
            }
            ("open", Some(url)) => {
                cx.hide();
                (self.env.open_url)(url);
                Outcome::Hide
            }
            ("copy", _) => self.activate(id, cx),
            _ => Outcome::Stay(None),
        }
    }

    fn hotkeys(&self) -> Vec<Binding> {
        let s = &self.settings;
        let bound = [(&s.hotkey, RECENT), (&s.card_hotkey, CARD), (&s.chat_hotkey, CHAT)];
        let spec = |(s, key): (&Option<String>, &str)| {
            let s = s.as_ref().filter(|s| !s.trim().is_empty())?;
            Some(Binding { spec: s.clone(), key: Ok(key.into()) })
        };
        bound.into_iter().filter_map(spec).collect()
    }

    /// `card_hotkey` gives the keyboard back when a card holds it, else moves it into the
    /// newest card; `chat_hotkey` shows or hides the chat window. The launcher stays as it is.
    fn hotkey(&mut self, key: &str, cx: &mut Cx) -> Option<ListView> {
        if key == CARD && !(self.env.unfocus)() {
            (self.env.focus)();
        }
        if key == CHAT {
            self.chat_toggle(cx);
        }
        (key == RECENT).then(|| recent_view(&self.settings.name))
    }

    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        let result = self.run_verb(args, cx);
        self.announce(cx);
        self.chat_refresh(cx);
        result
    }

    fn verbs(&self) -> &'static str {
        "message post [--title t] [--url u] [--reply-to id] [--id id] [--thread t] [--pending|--partial] <body...> | message ls [--limit n] | message threads [--limit n] | message thread <t> [--limit n] | message chat [--thread t] [--snapshot <png>] | message ask [--thread t] <text...> | message show [id] | message hide | message card post <json>|--stdin | message card get|show <id> | message card ls [--limit n] | message card dismiss <id>|--all | message card spec | message card press <id> <action> [values-json] | message card focus"
    }
}
