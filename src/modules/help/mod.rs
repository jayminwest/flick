//! Module `help`: Flick Help, a how-to for each feature (`topics.rs`). Ids are `help:index`
//! (the root item **Flick Help**, view `topics`), `help:topic/<slug>` (rows of `topics`) and
//! `help:key/<slug>/<n>` (rows of `keys/<slug>`, a topic's keys). ↵ on a topic opens its
//! `docs/` page on GitHub, or its key view when it has keys; ⌘K: Open Docs, Copy How-To. Views do
//! not record use. Table `[help]` has only `enabled`.

mod topics;
mod wire;

use crate::core::{Action, Cx, Icon, Item, ItemId, ListView, Module, Outcome};
use topics::{TOPICS, Topic};
use wire::Hooks;

const ID: &str = "help";
const TOPIC: &str = "topic/";
const KEY: &str = "key/";
const KEYS_VIEW: &str = "keys/";

#[derive(Default)]
pub struct Help {
    hooks: Hooks,
}

/// The topic a `topic/<slug>` or `key/<slug>/<n>` id names.
fn topic_of(key: &str) -> Option<&'static Topic> {
    let slug = key.strip_prefix(TOPIC).or_else(|| key.strip_prefix(KEY)?.split('/').next())?;
    topics::find(slug)
}

fn topic_item(t: &Topic) -> Item {
    let verb = if t.keys.is_some() { "Show Keys" } else { "Open Docs" };
    Item {
        subtitle: t.how.into(),
        keywords: vec![t.slug.into()],
        ..Item::new(ItemId::new(ID, format!("{TOPIC}{}", t.slug)), t.title, verb, Icon::Symbol(t.symbol))
    }
}

fn key_items(t: &Topic) -> Vec<Item> {
    topics::keys(t)
        .into_iter()
        .enumerate()
        .map(|(i, k)| Item {
            accessory: k.press.into(),
            keywords: vec![k.press.into(), k.short.into()],
            ..Item::new(ItemId::new(ID, format!("{KEY}{}/{i}", t.slug)), k.long, "Open Docs", Icon::Symbol("keyboard"))
        })
        .collect()
}

impl Help {
    fn open_docs(&self, t: &Topic, cx: &mut Cx) -> Outcome {
        cx.hide();
        (self.hooks.open)(&topics::url(t));
        Outcome::Hide
    }
}

impl Module for Help {
    fn id(&self) -> &'static str {
        ID
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        // Every topic title is a keyword, so "draw help" finds this.
        let titles = TOPICS.iter().map(|t| t.title).collect::<Vec<_>>().join(" ");
        vec![Item {
            subtitle: "How each feature works, and the draw keys".into(),
            accessory: "Command".into(),
            keywords: vec!["tutorial guide docs how to keys shortcuts ?".into(), titles],
            ..Item::new(ItemId::new(ID, "index"), "Flick Help", "Open Help", Icon::Symbol("questionmark.circle"))
        }]
    }

    /// Views `topics` and `keys/<slug>`.
    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        if view == "topics" {
            return Some(ListView {
                placeholder: "Search help…".into(),
                footer: "Flick Help  ·  ⌘K to copy  ·  esc to go back".into(),
                ..ListView::new(ID, view)
            });
        }
        let t = topics::find(view.strip_prefix(KEYS_VIEW)?).filter(|t| t.keys.is_some())?;
        Some(ListView {
            placeholder: format!("{} keys…", t.title),
            footer: format!("{}  ·  ? shows these keys on screen  ·  esc to go back", t.title),
            ..ListView::new(ID, view)
        })
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        let items = match view.name.strip_prefix(KEYS_VIEW).and_then(topics::find) {
            Some(t) => key_items(t),
            None => TOPICS.iter().map(topic_item).collect(),
        };
        // Keep the listed order for equal scores.
        let n = items.len();
        view.items = cx.ranker.rank(cx.query, items, |i| items_pos(i, n));
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        let key = id.key();
        if key == "index" {
            return Outcome::Push(ListView::new(ID, "topics"));
        }
        match topic_of(key) {
            Some(t) if t.keys.is_some() && key.starts_with(TOPIC) => {
                Outcome::Push(ListView::new(ID, format!("{KEYS_VIEW}{}", t.slug)))
            }
            Some(t) => self.open_docs(t, cx),
            None => Outcome::Stay(None),
        }
    }

    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        if topic_of(id.key()).is_none() {
            return vec![];
        }
        vec![
            Action::new("docs", "Open Docs", Icon::Symbol("book")),
            Action::new("copy", "Copy How-To", Icon::Symbol("doc.on.doc")),
        ]
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        let Some(t) = topic_of(id.key()) else { return Outcome::Stay(None) };
        match key {
            "docs" => self.open_docs(t, cx),
            "copy" => {
                (self.hooks.copy)(&topics::text(t));
                Outcome::Stay(Some(format!("Copied {}", t.title)))
            }
            _ => Outcome::Stay(None),
        }
    }
}

/// An item's place in its list, as a tiny rank bonus: earlier rows first.
fn items_pos(item: &Item, n: usize) -> f64 {
    let key = item.id.key();
    let pos = match key.strip_prefix(KEY) {
        Some(rest) => rest.rsplit('/').next().and_then(|i| i.parse().ok()),
        None => key.strip_prefix(TOPIC).and_then(|s| TOPICS.iter().position(|t| t.slug == s)),
    };
    -(pos.unwrap_or(n) as f64) * 1e-3
}

#[cfg(test)]
mod tests;
