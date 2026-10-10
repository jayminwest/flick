//! The launcher side of the chats: the root item `llm:chat`, the `models` view (every normal
//! server's models, `llm:model:<server>/<model>`) and the `threads` view (the kept chats,
//! `llm:thread:<id>`, ⌘K Delete Chat asks first, flick-5dfe) of the normal chat
//! (flick-6a0d); with no normal server there is no
//! `llm:chat`. The root item `llm:private` opens the private chat (flick-c325), shown with
//! any server; with no private server it says `NO_PRIVATE`, as does the `private` view the
//! private hotkey shows then.

use super::Llm;
use super::io::{self, Models};
use super::settings::NO_PRIVATE;
use super::store::Chats;
use crate::core::{Action, Confirm, ConfirmRow, Cx, Icon, Item, ItemId, ListView, Outcome};

/// The root item's key.
pub const CHAT: &str = "chat";
/// The model picker's view name (and the root item's action that opens it).
pub const MODELS: &str = "models";
/// The thread list's view name (and the root item's action that opens it).
pub const THREADS: &str = "threads";
/// The private chat's root item key, and the view that says why it cannot open.
pub const PRIVATE: &str = "private";
const MODEL: &str = "model:";
const THREAD: &str = "thread:";
const RETRY: &str = "retry:";
/// A thread item's action, and the prefix of its confirmation's token.
const DELETE: &str = "delete";

fn id(key: impl std::fmt::Display) -> ItemId {
    ItemId::new(super::ID, key)
}

/// Rank `items` for `cx.query`, keeping their order for equal scores.
fn ranked(items: Vec<Item>, cx: &mut Cx) -> Vec<Item> {
    let order: Vec<String> = items.iter().map(|i| i.id.to_string()).collect();
    cx.ranker.rank(cx.query, items, |i| {
        let pos = order.iter().position(|o| *o == i.id.as_str()).unwrap_or(order.len());
        -(pos as f64) * 1e-3
    })
}

impl Llm {
    /// The root items: `llm:chat` with a normal server, `llm:private` with any server.
    pub(super) fn root_items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = self.chat_item().into_iter().collect();
        if !self.settings.servers.is_empty() {
            let subtitle = match self.settings.private(None) {
                Ok(s) => format!("{} · nothing is saved", s.name),
                Err(_) => "No private server ([[llm.servers]] private = true)".into(),
            };
            items.push(Item {
                subtitle,
                accessory: "Command".into(),
                keywords: ["llm", "model", "chat", "ai", "local", "private", "incognito"].map(String::from).to_vec(),
                ..Item::new(id(PRIVATE), "Private Model Chat", "Open Private Chat", Icon::Symbol("lock"))
            });
        }
        items
    }

    fn chat_item(&self) -> Option<Item> {
        let server = self.settings.normal(None).ok()?;
        let (server, model) = self.picked().unwrap_or((server.name.as_str(), self.settings.default_model.as_str()));
        let model = if model.is_empty() { "first listed model" } else { model };
        Some(Item {
            subtitle: format!("{server} · {model}"),
            accessory: "Command".into(),
            keywords: ["llm", "model", "chat", "ai", "local"].map(String::from).to_vec(),
            ..Item::new(id(CHAT), "Local Model Chat", "Open Chat", Icon::Symbol("brain"))
        })
    }

    /// ⌘K: the root item's views; a kept thread's Delete Chat.
    pub(super) fn item_actions(item: &ItemId) -> Vec<Action> {
        match item.key() {
            CHAT => vec![
                Action::new(MODELS, "Choose Model…", Icon::Symbol("cpu")),
                Action::new(THREADS, "Chat History", Icon::Symbol("clock.arrow.circlepath")),
            ],
            k if k.starts_with(THREAD) => vec![Action::new(DELETE, "Delete Chat", Icon::Symbol("trash"))],
            _ => vec![],
        }
    }

    /// A ⌘K action: open a view, or ask before deleting a kept thread.
    pub(super) fn item_act(item: &ItemId, key: &str, cx: &Cx) -> Outcome {
        if !Llm::item_actions(item).iter().any(|a| a.key == key) {
            return Outcome::Stay(None);
        }
        let Some(id) = item.key().strip_prefix(THREAD) else { return Outcome::Push(ListView::new(super::ID, key)) };
        let Some((t, _)) = cx.store.llm_thread(id) else { return Outcome::Stay(Some(format!("Chat {id} is no longer kept"))) };
        let s = if t.messages == 1 { "" } else { "s" };
        Outcome::Confirm(Confirm {
            rows: vec![ConfirmRow { subtitle: format!("{} · {}  ·  {} message{s}", t.server, t.model, t.messages), ..ConfirmRow::new(&t.title) }],
            label: "Delete Chat".into(),
            destructive: true,
            ..Confirm::new(super::ID, format!("{DELETE}:{id}"), format!("Delete chat \"{}\"?", t.title))
        })
    }

    /// Delete Chat confirmed: drop the thread from flick.db; if the window shows it, a reply in
    /// flight is stopped first and the window starts a new chat.
    pub(super) fn delete_confirmed(&mut self, token: &str, cx: &Cx) -> Outcome {
        let Some(id) = token.strip_prefix(DELETE).and_then(|t| t.strip_prefix(':')) else { return Outcome::Stay(None) };
        let shown = self.chat.thread.as_ref().is_some_and(|t| t.id == id);
        if shown {
            self.chat_forget(cx);
        }
        Outcome::Stay(Some(match cx.store.llm_delete(id) {
            Ok(true) => "Deleted the chat".into(),
            Ok(false) => format!("Chat {id} is no longer kept"),
            Err(e) => e,
        }))
    }

    /// A view by name; opening `models` asks every normal server for its list again.
    pub(super) fn view(&self, name: &str) -> Option<ListView> {
        match name {
            MODELS => {
                for s in self.settings.servers.iter().filter(|s| !s.private) {
                    io::fetch_models(&self.shared, s, self.hooks);
                }
                Some(ListView {
                    placeholder: "Search models…".into(),
                    footer: "Models  ·  ↵ chats with it".into(),
                    empty: "No normal server in [[llm.servers]]".into(),
                    ..ListView::new(super::ID, MODELS)
                })
            }
            PRIVATE => Some(ListView { empty: NO_PRIVATE.into(), escape_hides: true, ..ListView::new(super::ID, PRIVATE) }),
            THREADS => Some(ListView {
                placeholder: "Search chats…".into(),
                footer: "Local model chats  ·  ↵ opens".into(),
                empty: if self.settings.history { "No chats kept yet" } else { "History is off ([llm] history = false)" }.into(),
                ..ListView::new(super::ID, THREADS)
            }),
            _ => None,
        }
    }

    pub(super) fn fill(&self, view: &mut ListView, cx: &mut Cx) {
        let items = match view.name.as_str() {
            MODELS => self.model_items(),
            THREADS => self.thread_items(cx),
            _ => vec![],
        };
        view.items = ranked(items, cx);
    }

    fn model_items(&self) -> Vec<Item> {
        let st = self.shared.lock();
        let mut items = vec![];
        for s in self.settings.servers.iter().filter(|s| !s.private) {
            let m: Models = st.models.get(&s.name).cloned().unwrap_or_default();
            let row = |key: String, title: String, verb| Item::new(id(key), title, verb, Icon::Symbol("server.rack"));
            match m.result {
                None => items.push(row(format!("{RETRY}{}", s.name), format!("{}: loading models…", s.name), "Retry")),
                Some(Err(e)) => items.push(row(format!("{RETRY}{}", s.name), format!("{}: {e}", s.name), "Retry")),
                Some(Ok(list)) => items.extend(list.into_iter().map(|model| {
                    let current = self.picked().is_some_and(|(ps, pm)| ps == s.name && pm == model.id);
                    let mut about = vec![s.name.clone()];
                    about.extend(model.state.clone());
                    about.extend(model.context_length.map(|c| format!("ctx {c}")));
                    Item {
                        subtitle: about.join(" · "),
                        accessory: if current { "Current" } else { "" }.into(),
                        keywords: vec![s.name.clone()],
                        ..Item::new(id(format!("{MODEL}{}/{}", s.name, model.id)), model.id, "Chat with Model", Icon::Symbol("cpu"))
                    }
                })),
            }
        }
        items
    }

    fn thread_items(&self, cx: &Cx) -> Vec<Item> {
        if !self.settings.history {
            return vec![];
        }
        cx.store
            .llm_threads(self.settings.max_threads)
            .into_iter()
            .map(|t| {
                let s = if t.messages == 1 { "" } else { "s" };
                Item {
                    subtitle: format!("{} · {}  ·  {} message{s}", t.server, t.model, t.messages),
                    keywords: vec![t.model.clone()],
                    ..Item::new(id(format!("{THREAD}{}", t.id)), t.title, "Open Chat", Icon::Symbol("bubble.left.and.bubble.right"))
                }
            })
            .collect()
    }

    /// Enter on any of the module's items.
    pub(super) fn enter(&mut self, item: &ItemId, cx: &mut Cx) -> Outcome {
        let key = item.key();
        if let Some(name) = key.strip_prefix(RETRY) {
            if let Ok(s) = self.settings.normal(Some(name)) {
                io::fetch_models(&self.shared, s, self.hooks);
            }
            return Outcome::Stay(Some(format!("Asking {name} for its models…")));
        }
        if let Some(rest) = key.strip_prefix(MODEL) {
            let mut normal = self.settings.servers.iter().filter(|s| !s.private);
            let found = normal.find_map(|s| Some((s.name.clone(), rest.strip_prefix(&format!("{}/", s.name))?)));
            let Some((server, model)) = found else { return Outcome::Stay(None) };
            cx.hide();
            self.pick(&server, model, cx);
            self.summon(None, cx);
            return Outcome::Hide;
        }
        if key == PRIVATE {
            if let Err(e) = self.settings.private(None) {
                return Outcome::Stay(Some(e));
            }
            cx.hide();
            // The gate passed, so the summon does not fail.
            return self.private_summon().err().map(Some).map_or(Outcome::Hide, Outcome::Stay);
        }
        let thread = key.strip_prefix(THREAD);
        if thread.is_none() && key != CHAT {
            return Outcome::Stay(None);
        }
        cx.hide();
        self.summon(thread, cx);
        Outcome::Hide
    }
}
