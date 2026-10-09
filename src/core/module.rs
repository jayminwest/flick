//! The module contract: one feature behind a narrow interface.

use super::store::Store;
use super::{Action, Event, Form, Item, ItemId, ListView, Outcome, Ranker};
use crate::config::Section;

/// A global hotkey a module asks for: `spec` (e.g. "cmd+Space") runs the module's `hotkey`
/// with `key`. `Err` reports a binding the module can't map, e.g. an unknown action name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub spec: String,
    pub key: Result<String, String>,
}

/// What a module may use while it handles a call.
pub struct Cx<'a> {
    /// The text in the search field.
    pub query: &'a str,
    pub store: &'a Store,
    pub ranker: &'a mut Ranker,
    /// Hides the launcher now. Call it before an action that needs the previous app
    /// frontmost (opening, focusing, pasting), then return `Outcome::Hide`.
    pub hide: fn(),
    /// The control request asked for structured output (`--json`): `command` may answer
    /// with a JSON object or array. False for events, hotkeys and the launcher.
    pub json: bool,
    /// The control request came from a session that may send its output to a remote model
    /// (`--remote`). False for events, hotkeys and the launcher.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the activity remote grant, flick-eed3")
    )]
    pub remote: bool,
}

impl Cx<'_> {
    pub fn hide(&self) {
        (self.hide)();
    }
}

/// Run `f` with a context over an in-memory store and a no-op `hide`.
#[cfg(test)]
pub fn test_cx<R>(query: &str, f: impl FnOnce(&mut Cx) -> R) -> R {
    let store = Store::in_memory();
    let mut ranker = Ranker::new();
    f(&mut Cx {
        query,
        store: &store,
        ranker: &mut ranker,
        hide: || {},
        json: false,
        remote: false,
    })
}

/// A feature. Every method has a do-nothing default, so a module implements only what it
/// needs.
pub trait Module {
    /// The `module` part of every `ItemId` this module creates.
    fn id(&self) -> &'static str;

    /// This module's store migrations, keyed by `id()` in `schema_versions`. Append only.
    /// The module's SQL touches only the tables these create.
    fn migrations(&self) -> &'static [&'static str] {
        &[]
    }

    /// Items this module contributes to root search.
    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        vec![]
    }

    /// Items placed above the ranked root results, unranked: exact matches such as
    /// "<keyword> <text>" for a quicklink.
    fn direct(&mut self, _cx: &mut Cx) -> Vec<Item> {
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

    /// Build this module's form `name` (a separate namespace from list views). `None` when
    /// it has no such form.
    fn form(&mut self, _name: &str, _cx: &mut Cx) -> Option<Form> {
        None
    }

    /// Save `form` (one this module built), its required fields already filled. `Ok` is a
    /// status for root search; `Err` is shown in the form, which stays open.
    fn submit(&mut self, form: &Form, _cx: &mut Cx) -> Result<String, String> {
        Err(format!("{}: cannot submit form \"{}\"", self.id(), form.name))
    }

    /// The action menu (cmd+K) for item `id`, asked each time the menu opens so state such
    /// as "running" is current. Empty when the item has none.
    fn actions(&mut self, _id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        vec![]
    }

    /// Run action `key` (one of `actions(id)`) on item `id`.
    fn act(&mut self, _id: &ItemId, _key: &str, _cx: &mut Cx) -> Outcome {
        Outcome::Stay(None)
    }

    /// The user confirmed this module's `Confirm` carrying `token`.
    fn confirmed(&mut self, _token: &str, _cx: &mut Cx) -> Outcome {
        Outcome::Stay(None)
    }

    /// Handle `event`. True when this module's views now show stale data, so a visible one
    /// refreshes.
    fn on_event(&mut self, _event: Event, _cx: &mut Cx) -> bool {
        false
    }

    /// Take settings from this module's `[<id>]` config table: at registration, and again
    /// on config reload (state such as history is kept). `Err` names what is wrong.
    fn configure(&mut self, _table: &Section) -> Result<(), String> {
        Ok(())
    }

    /// Global hotkeys this module wants, from its config table.
    fn hotkeys(&self) -> Vec<Binding> {
        vec![]
    }

    /// Run the hotkey bound to `key`. `Some(view)` toggles the launcher on that view (shows
    /// it, or hides the launcher when it already shows it); `None` leaves the launcher alone.
    fn hotkey(&mut self, _key: &str, _cx: &mut Cx) -> Option<ListView> {
        None
    }

    /// Run a control command (`flick <id> <verb> [args...]`); `args` starts at the verb.
    /// `Ok` is the text to print, `Err` a message for the caller.
    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        Err(unknown_verb(self.id(), args))
    }

    /// `command`'s verbs for `flick help`, on one line, e.g. `toy ping | toy get <key>`.
    /// Empty when the module has no verbs.
    fn verbs(&self) -> &'static str {
        ""
    }
}

/// The error for a verb module `id` doesn't have.
pub fn unknown_verb(id: &str, args: &[String]) -> String {
    match args.first() {
        Some(verb) => format!("{id}: unknown command \"{verb}\""),
        None => format!("{id}: missing command"),
    }
}
