//! Actions: Enter runs an item's first action, ⌘K lists them all, Tab runs the one its `tab`
//! names. An item is looked up in what the screen shows: the view on it when that is one of
//! the item's extension, else that extension's root items. One no longer there (the cache
//! moved on) says so in the footer and asks for the root items again.
//!
//! An action with `confirm` asks first (`Outcome::Confirm`, its title on the button). Then:
//!
//! - `ext` sends `op: act` on the extension's worker, and `after` applies at once
//!   (optimistic): `hide` hides the launcher, `root` goes back to root search, `stay` stays,
//!   `<title>…` in the footer of the view it ran in. The reply's `status` replaces that
//!   footer while the view shows, else it is a HUD toast; `refresh` asks for the root items
//!   and the view again. An error is logged and always toasted (even after a hide), shows
//!   in the footer while the view shows, and asks for both again (an item may be gone).
//! - `push` and `form` open the extension's view or form: no call besides its `open` or
//!   `form`.
//! - A card `do` runs as a local card's would: `open_url`, `open_app` and `copy` at once
//!   (the launcher hidden first for `after: hide`), `script` and `flick` as Flick's own CLI
//!   on a thread, answered like `op: act`; `dismiss` hides the launcher.

use super::proto::item::{After, ExtAction, ExtItem, Run};
use super::proto::{Done, Reply, Request};
use super::{Ext, items, worker};
use crate::core::card::Do;
use crate::core::{Action, Confirm, Cx, Icon, ItemId, ListView, Outcome};

/// The icon of an action that names none.
pub const ICON: &str = "bolt";

/// An action waiting for the user to confirm it.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    /// `Confirm::token`: `<ext>/<item>#<action>`.
    pub token: String,
    pub run: Ran,
    pub action: ExtAction,
}

/// Where an action runs: the extension, the view (None: root search) and the item key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    pub ext: String,
    pub view: Option<String>,
    pub item: String,
}

impl Ext {
    /// Item `id` as the screen shows it, with where it shows.
    fn find(&self, id: &ItemId) -> Option<(Ran, &ExtItem)> {
        let (ext, key) = id.key().split_once('/')?;
        let showing = self.showing.as_deref();
        let view = showing.and_then(items::split).filter(|(e, _)| *e == ext).map(|(_, v)| v);
        let list = match view {
            Some(_) => &self.views.get(showing?)?.items,
            None => self.root.get(ext)?,
        };
        let item = list.iter().find(|i| i.key == key)?;
        let ran = Ran { ext: ext.into(), view: view.map(String::from), item: key.into() };
        Some((ran, item))
    }

    /// Item `id`'s actions, for ⌘K.
    pub(super) fn actions_of(&self, id: &ItemId) -> Vec<Action> {
        let Some((_, item)) = self.find(id) else { return vec![] };
        let icon = |a: &ExtAction| a.icon.clone().unwrap_or_else(|| Icon::symbol(ICON));
        item.actions.iter().map(|a| Action::new(a.key.clone(), a.title.clone(), icon(a))).collect()
    }

    /// Run item `id`'s action `key` (None: its first), asking first when it has `confirm`.
    pub(super) fn act_on(&mut self, id: &ItemId, key: Option<&str>, cx: &mut Cx) -> Outcome {
        let found = self.find(id).map(|(ran, item)| {
            (ran, item.actions.iter().find(|a| key.is_none_or(|k| a.key == k)).cloned())
        });
        let Some((ran, Some(action))) = found else {
            let ext = id.key().split_once('/').map_or("ext", |(ext, _)| ext).to_string();
            self.fetch(&ext, Request::Root);
            return Outcome::Stay(Some(format!("{ext}: that item is gone; try again")));
        };
        let Some(question) = action.confirm.clone() else {
            return self.run(ran, &action, cx);
        };
        let token = format!("{}/{}#{}", ran.ext, ran.item, action.key);
        let confirm = Confirm { label: action.title.clone(), ..Confirm::new("ext", token.clone(), question) };
        self.asked = Some(Asked { token, run: ran, action });
        Outcome::Confirm(confirm)
    }

    /// The user confirmed the action that asked with `token`.
    pub(super) fn confirm_asked(&mut self, token: &str, cx: &mut Cx) -> Outcome {
        match self.asked.take() {
            Some(asked) if asked.token == token => self.run(asked.run, &asked.action, cx),
            _ => Outcome::Stay(None),
        }
    }

    fn run(&mut self, ran: Ran, action: &ExtAction, cx: &mut Cx) -> Outcome {
        let Ran { ext, view, item } = ran;
        let request = Request::Act { view: view.clone(), item, action: action.key.clone() };
        let (job, inbox) = (self.hooks.job, self.inbox.clone());
        let started = match &action.run {
            Run::Push(v) => return Outcome::Push(ListView::new("ext", format!("{ext}/{v}"))),
            Run::Form(f) => return Outcome::Form { module: "ext", name: format!("{ext}/{f}") },
            Run::Card(Do::Dismiss) => return Outcome::Hide,
            Run::Ext => self.call(&ext, request),
            Run::Card(Do::Script { name, query }) => {
                let words = ["script", "run", name].into_iter().map(String::from);
                worker::flick(job, inbox, &ext, request, words.chain(query.clone()).collect())
            }
            Run::Card(Do::Flick(words)) => worker::flick(job, inbox, &ext, request, words.clone()),
            Run::Card(other) => {
                if action.after == After::Hide {
                    cx.hide();
                }
                let done = (self.hooks.card)(other).map_err(|e| format!("{ext}: {e}"));
                if let Err(e) = done {
                    self.report(&ext, &e);
                }
                return self.after(action.after, None);
            }
        };
        if let Err(e) = started {
            self.report(&ext, &e);
            return Outcome::Stay(Some(e));
        }
        let running = view.map(|v| (format!("{ext}/{v}"), format!("{}…", action.title)));
        self.after(action.after, running)
    }

    /// What the launcher does once an action started; `running` is the footer of the view
    /// it ran in until the answer.
    fn after(&mut self, after: After, running: Option<(String, String)>) -> Outcome {
        match after {
            After::Hide => Outcome::Hide,
            After::Root => Outcome::Root,
            After::Stay => {
                if running.is_some() {
                    self.status = running;
                }
                Outcome::Stay(None)
            }
        }
    }

    /// Extension `ext`'s answer to an action run in `view`. True: the footer of the view on
    /// the screen changed.
    pub(super) fn acted(&mut self, ext: &str, view: Option<&str>, result: Result<Reply, String>) -> bool {
        let name = view.map(|v| format!("{ext}/{v}"));
        let shown = name.filter(|n| self.showing.as_ref() == Some(n) && (self.hooks.visible)());
        // An error toasts here; a status only when no footer shows it.
        let (line, again) = match result {
            Ok(Reply::Done(Done { status, refresh })) => (status.map(Ok), refresh),
            Ok(_) => (None, false),
            Err(e) => {
                self.report(ext, &e);
                (Some(Err(e)), true)
            }
        };
        if again {
            self.refetch(ext);
        }
        let Some(view) = shown else {
            if let Some(Ok(status)) = &line {
                (self.hooks.toast)(ext, status);
            }
            return false;
        };
        self.status = line.map(|line| (view, line.unwrap_or_else(|e| e)));
        true
    }

    /// Log `error` and toast it.
    pub(super) fn report(&self, ext: &str, error: &str) {
        (self.hooks.log)(error);
        (self.hooks.toast)(ext, error);
    }

    /// Ask extension `ext` for its root items again, and for the view on the screen when it
    /// is one of its.
    pub(super) fn refetch(&mut self, ext: &str) {
        self.fetch(ext, Request::Root);
        let view = self.showing.as_deref().and_then(items::split).filter(|(e, _)| *e == ext);
        if let Some((_, view)) = view {
            let view = view.to_string();
            self.fetch(ext, Request::Open { view });
        }
    }
}

#[cfg(test)]
mod tests;
