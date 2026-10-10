//! Module `message`: short messages posted to this Mac, usually by an agent on another one
//! (`flick --host <mac> message post ...`), shown in a small corner panel that does not take
//! focus (`platform::hud`), as a notification, or both, and kept in a history list. A post
//! may be `--pending` (a placeholder, e.g. "sent to KOTA") that a later post with
//! `--reply-to <its id>` replaces. Ids are `message:list` (root item, view `recent`) and
//! `message:<message id>` (rows of `recent`; Enter copies the body, ⌘K Show / Open Link /
//! Copy). Table `[message]`: `name`, `style`, `position`, `width`, `timeout_secs`,
//! `max_cards`, `max_history`, `sound`, `hotkey` (opens `recent`). Table `messages` holds the
//! history. Each message shows as its own card, keyed by its id. `message card <verb>`
//! posts and manages structured cards (`card.rs`, `core::card`), stored in the same table.

mod card;
pub mod store;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_card;
mod text;
mod wire;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::config::Section;
use crate::core::card::State;
use crate::core::{Action, Binding, Cx, Icon, Item, ItemId, ListView, Module, Outcome, unknown_verb};
use crate::platform::hud::{Content, Corner, Options, Placement, TextCard};
use store::{Message, Messages};
use wire::Env;

const LIST: &str = "list";
const RECENT: &str = "recent";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Style {
    /// The corner panel.
    #[default]
    Panel,
    /// A system notification.
    Notification,
    Both,
    /// History only.
    None,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Position {
    #[default]
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
    Top,
    Bottom,
}

impl Position {
    fn corner(self) -> Corner {
        match self {
            Position::TopRight => Corner::TopRight,
            Position::TopLeft => Corner::TopLeft,
            Position::BottomRight => Corner::BottomRight,
            Position::BottomLeft => Corner::BottomLeft,
            Position::Top => Corner::Top,
            Position::Bottom => Corner::Bottom,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    /// The root item's title and the panel header of a post with no `--title`.
    name: String,
    style: Style,
    position: Position,
    /// Panel width in points, 240 to 900.
    width: f64,
    /// Seconds the panel stays; 0 keeps it until dismissed.
    timeout_secs: u64,
    /// Cards shown at once; older ones collapse into a `+N more` pill.
    max_cards: usize,
    /// Messages kept in history.
    max_history: usize,
    /// Play a short sound when a reply arrives (never for pending posts).
    sound: bool,
    /// Opens the message list.
    hotkey: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: "Messages".into(),
            style: Style::Panel,
            position: Position::TopRight,
            width: 380.0,
            timeout_secs: 20,
            max_cards: 4,
            max_history: 50,
            sound: true,
            hotkey: None,
        }
    }
}

#[derive(Default)]
pub struct Inbox {
    env: Env,
    settings: Settings,
    /// Cards dismissed through `card dismiss`; a `done` update of one stays in history.
    dismissed: HashSet<String>,
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

    /// A message card: timed out like any card, no sound for a placeholder.
    fn options(&self, m: &Message) -> Options {
        Options {
            timeout_secs: self.settings.timeout_secs as f64,
            sound: self.settings.sound && !m.pending,
            sticky: false,
        }
    }

    fn header<'a>(&'a self, m: &'a Message) -> &'a str {
        m.title.as_deref().unwrap_or(&self.settings.name)
    }

    /// Show `m` the configured way; `panel_only` skips the notification (a re-show). A
    /// card shows as text: its title as the header, the rest of `plain(card)` as the body.
    fn display(&self, m: &Message, panel_only: bool) {
        let c = card::stored(m);
        let body = c.as_ref().map_or_else(|| text::plain(&m.body), card::body);
        let header = c.as_ref().map_or_else(|| self.header(m), |c| c.title.as_str());
        let opts = c.as_ref().map_or_else(|| self.options(m), |c| self.card_options(c));
        let pending = m.pending || c.as_ref().is_some_and(|c| c.state == State::Pending);
        let style = self.settings.style;
        if matches!(style, Style::Panel | Style::Both) || panel_only {
            let context = m.context.as_deref().map(|c| format!("Re: {}", text::preview(c, 80)));
            let time = text::stamp(m.ts, (self.env.now)(), (self.env.utc_offset)(m.ts));
            let card = TextCard {
                header,
                time: &time,
                context: context.as_deref().unwrap_or(""),
                body: &text::clip(&body),
                link: m.url.as_deref(),
                pending,
            };
            (self.env.show)(&m.id, &Content::Text(card), &self.placement(), &opts);
        }
        if matches!(style, Style::Notification | Style::Both) && !panel_only && !pending {
            (self.env.notify)(&m.id, header, &body);
        }
    }

    /// Save `m`; `took` (its `reply_to` was a pending message, now gone) also removes that
    /// placeholder's card, which `m` takes the place of.
    fn save(&self, m: &Message, took: bool, cx: &Cx) -> Result<(), String> {
        cx.store.put_message(m, self.settings.max_history)?;
        if let Some(pending) = m.reply_to.as_deref().filter(|r| took && *r != m.id) {
            (self.env.dismiss)(pending);
        }
        Ok(())
    }

    /// Save and show a post; returns its id and whether it replaced a pending message.
    fn post(&self, args: &[String], cx: &Cx) -> Result<(String, bool), String> {
        let post = text::parse_post(args)?;
        let id = post.id.unwrap_or_else(self.env.new_id);
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
            ..Message::default()
        };
        self.save(&m, replaced, cx)?;
        self.display(&m, false);
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

    /// `ls` text: one line per message, `id  time  [title: ]body`.
    fn ls(&self, list: &[Message], now: i64) -> String {
        let line = |m: &Message| {
            let time = text::stamp(m.ts, now, (self.env.utc_offset)(m.ts));
            let title = m.title.as_ref().map(|t| format!("{t}: ")).unwrap_or_default();
            let pending = if m.pending { " (pending)" } else { "" };
            let body = text::preview(&text::plain(&m.body), 100);
            format!("{}\t{time}\t{title}{body}{pending}", m.id)
        };
        list.iter().map(line).collect::<Vec<_>>().join("\n")
    }

    fn run_verb(&mut self, args: &[String], cx: &Cx) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["card", ..] => self.card_verb(&args[1..], cx),
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
        if s.max_history == 0 {
            return Err("[message]: max_history must be at least 1".into());
        }
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
        (view == RECENT).then(|| recent_view(&self.settings.name))
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
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
        match cx.store.message(id.key()) {
            Some(m) => {
                (self.env.copy)(&text::plain(&m.body));
                Outcome::Stay(Some("Copied message".into()))
            }
            None => Outcome::Stay(None),
        }
    }

    fn actions(&mut self, id: &ItemId, cx: &mut Cx) -> Vec<Action> {
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
        let spec = self.settings.hotkey.as_ref().filter(|s| !s.trim().is_empty());
        spec.map(|spec| Binding { spec: spec.clone(), key: Ok(RECENT.into()) }).into_iter().collect()
    }

    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        (key == RECENT).then(|| recent_view(&self.settings.name))
    }

    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        self.run_verb(args, cx)
    }

    fn verbs(&self) -> &'static str {
        "message post [--title t] [--url u] [--reply-to id] [--id id] [--pending] <body...> | message ls [--limit n] | message show [id] | message hide | message card post <json>|--stdin | message card get|show <id> | message card ls [--limit n] | message card dismiss <id>|--all"
    }
}
