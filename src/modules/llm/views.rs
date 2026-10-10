//! The launcher side of the normal chat (flick-6a0d): the root item `llm:chat`, the `models`
//! view (every normal server's models, `llm:model:<server>/<model>`) and the `threads` view
//! (the kept chats, `llm:thread:<id>`). With no normal server there is no item and nothing
//! runs.

use super::Llm;
use super::io::{self, Models};
use super::store::Chats;
use crate::core::{Action, Cx, Icon, Item, ItemId, ListView, Outcome};

/// The root item's key.
pub const CHAT: &str = "chat";
/// The model picker's view name (and the root item's action that opens it).
pub const MODELS: &str = "models";
/// The thread list's view name (and the root item's action that opens it).
pub const THREADS: &str = "threads";
const MODEL: &str = "model:";
const THREAD: &str = "thread:";
const RETRY: &str = "retry:";

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
    /// The root item, only with a normal server.
    pub(super) fn root_items(&self) -> Vec<Item> {
        let Ok(server) = self.settings.normal(None) else { return vec![] };
        let (server, model) = match &self.chat.pick {
            Some((s, m)) => (s.as_str(), m.as_str()),
            None => (server.name.as_str(), self.settings.default_model.as_str()),
        };
        let model = if model.is_empty() { "first listed model" } else { model };
        vec![Item {
            subtitle: format!("{server} · {model}"),
            accessory: "Command".into(),
            keywords: ["llm", "model", "chat", "ai", "local"].map(String::from).to_vec(),
            ..Item::new(id(CHAT), "Local Model Chat", "Open Chat", Icon::Symbol("brain"))
        }]
    }

    pub(super) fn root_actions(item: &ItemId) -> Vec<Action> {
        if item.key() != CHAT {
            return vec![];
        }
        vec![
            Action::new(MODELS, "Choose Model…", Icon::Symbol("cpu")),
            Action::new(THREADS, "Chat History", Icon::Symbol("clock.arrow.circlepath")),
        ]
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
        let items = if view.name == MODELS { self.model_items() } else { self.thread_items(cx) };
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
                    let current = self.chat.pick.as_ref().is_some_and(|(ps, pm)| *ps == s.name && *pm == model.id);
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
            let found = self.settings.servers.iter().find_map(|s| Some((s.name.clone(), rest.strip_prefix(&format!("{}/", s.name))?)));
            let Some((server, model)) = found else { return Outcome::Stay(None) };
            cx.hide();
            self.pick(&server, model);
            self.summon(None, cx);
            return Outcome::Hide;
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
