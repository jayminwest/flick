//! The launcher's `message/threads` view (flick-eedd): the chat threads, newest activity
//! first, each as `message:thread:<t>` (a thread id never holds `:`, so these never meet a
//! message id). Enter hides the launcher and opens the chat window on that thread. The view
//! opens from ⌘K on the root item (Chat Threads).

use super::model;
use crate::core::{Action, Cx, Icon, Item, ItemId, ListView, Outcome};
use crate::modules::message::store::Messages;
use crate::modules::message::{Inbox, text};

/// The view's name, and the root item's action key that opens it.
pub const VIEW: &str = "threads";
/// The item key prefix of a thread row.
const PREFIX: &str = "thread:";

pub fn view() -> ListView {
    ListView {
        placeholder: "Search chat threads…".into(),
        footer: "Chat threads  ·  ↵ opens".into(),
        empty: "No chat threads yet".into(),
        ..ListView::new("message", VIEW)
    }
}

/// The root item's action that opens the view.
pub fn action() -> Action {
    Action::new(VIEW, "Chat Threads", Icon::Symbol("bubble.left.and.text.bubble.right"))
}

/// The thread a row's item key names.
pub fn thread_of(key: &str) -> Option<&str> {
    key.strip_prefix(PREFIX)
}

impl Inbox {
    /// Fill the view for `cx.query`, keeping newest-first for equal scores.
    pub fn refresh_threads(&self, view: &mut ListView, cx: &mut Cx) {
        let now = (self.env.now)();
        let list = cx.store.threads(self.settings.chat_threads);
        let n = list.len();
        let items: Vec<Item> = list
            .iter()
            .map(|t| {
                let time = text::stamp(t.last_ts, now, (self.env.utc_offset)(t.last_ts));
                let count = format!("{} message{}", t.messages, if t.messages == 1 { "" } else { "s" });
                let id = ItemId::new("message", format!("{PREFIX}{}", t.id));
                Item {
                    subtitle: format!("{time}  ·  {count}"),
                    keywords: vec![t.id.clone()],
                    ..Item::new(id, model::title(&t.first_body), "Open Chat", Icon::Symbol("bubble.left.and.bubble.right"))
                }
            })
            .collect();
        let order: Vec<String> = items.iter().map(|i| i.id.to_string()).collect();
        view.items = cx.ranker.rank(cx.query, items, |i| {
            let pos = order.iter().position(|o| *o == i.id.as_str()).unwrap_or(n);
            -(pos as f64) * 1e-3
        });
    }

    /// Enter on a thread row: the launcher hides and the chat window shows the thread.
    pub fn open_thread(&mut self, thread: &str, cx: &mut Cx) -> Outcome {
        cx.hide();
        self.summon(Some(thread.to_string()), cx);
        Outcome::Hide
    }
}
