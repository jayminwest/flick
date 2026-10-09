//! The module contract: one feature behind a narrow interface.

use super::{Item, ItemId, ListView, Outcome, Ranker};
use crate::config::Config;
use crate::store::Store;

/// Something that happened outside any module. Dispatched to every module in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The launcher opened at root search.
    LauncherOpened,
}

/// What a module may use while it handles a call.
pub struct Cx<'a> {
    /// The text in the search field.
    pub query: &'a str,
    pub config: &'a mut Config,
    pub store: &'a Store,
    pub ranker: &'a mut Ranker,
    /// Hides the launcher now. Call it before an action that needs the previous app
    /// frontmost (opening, focusing, pasting), then return `Outcome::Hide`.
    pub hide: fn(),
}

impl Cx<'_> {
    pub fn hide(&self) {
        (self.hide)();
    }
}

/// Run `f` with a context over `config`, an in-memory store and a no-op `hide`.
#[cfg(test)]
pub fn test_cx<R>(query: &str, mut config: Config, f: impl FnOnce(&mut Cx) -> R) -> R {
    let store = Store::in_memory();
    let mut ranker = Ranker::new();
    f(&mut Cx { query, config: &mut config, store: &store, ranker: &mut ranker, hide: || {} })
}

/// A feature. Every method has a do-nothing default, so a module implements only what it
/// needs.
pub trait Module {
    /// The `module` part of every `ItemId` this module creates.
    fn id(&self) -> &'static str;

    /// Items this module contributes to root search.
    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        vec![]
    }

    /// Enter this module's list `view`. `None` when it has no such view.
    fn open(&mut self, _view: &str, _cx: &mut Cx) -> Option<ListView> {
        None
    }

    /// Fill `view` (one this module opened) for `cx.query`.
    fn refresh(&mut self, _view: &mut ListView, _cx: &mut Cx) {}

    /// Run the item `id` (whose `module()` is this module).
    fn activate(&mut self, _id: &ItemId, _cx: &mut Cx) -> Outcome {
        Outcome::Stay(None)
    }

    fn on_event(&mut self, _event: Event, _cx: &mut Cx) {}
}
