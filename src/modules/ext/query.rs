//! Searches the extension answers: a `filter: "ext"` view sends `op: query` with the search
//! text, and `<keyword> <text>` in root search, with one of an extension's `keywords`, sends
//! `op: keyword`; its items are direct rows, above the ranked results.
//!
//! Newest wins, per extension: each search text is sent as a query (`Worker::query`), which
//! supersedes the one before it on the worker (debounced there, the older child killed). An
//! answer counts only while it answers the newest query sent; until it lands the screen keeps
//! the last answer of the same view or keyword (stale while revalidate). An empty search
//! text, another screen or a reload stops the query (`Worker::cancel`). A failed query is
//! logged, and shown in the footer of its view while that is on the screen; it is never
//! posted, since it may fail at every keystroke.

use super::proto::item::ExtItem;
use super::proto::{Reply, Request};
use super::{Ext, items};
use crate::core::{Cx, Item};

/// `request` asks view `view` (`op: query`).
fn of_view(request: &Request, view: &str) -> bool {
    matches!(request, Request::Query { view: v, .. } if v == view)
}

/// `request` asks keyword `keyword` (`op: keyword`).
fn of_keyword(request: &Request, keyword: &str) -> bool {
    matches!(request, Request::Keyword { keyword: k, .. } if k == keyword)
}

impl Ext {
    /// Send query `request` to extension `ext`, unless it is the newest sent already and its
    /// worker still runs.
    fn ask(&mut self, ext: &str, request: Request) {
        let live = self.workers.iter().any(|w| w.spec().name == ext && w.alive());
        if live && self.sent_to(ext) == Some(&request) {
            return;
        }
        match self.worker(ext) {
            Ok(worker) => {
                worker.query(request.clone());
                self.sent.retain(|(n, _)| n != ext);
                self.sent.push((ext.to_string(), request));
            }
            Err(e) => (self.hooks.log)(&e),
        }
    }

    /// The newest query sent to extension `ext`.
    pub(super) fn sent_to(&self, ext: &str) -> Option<&Request> {
        self.sent.iter().find(|(n, _)| n == ext).map(|(_, r)| r)
    }

    /// Stop extension `ext`'s query: a running one is killed, its answer dropped.
    pub(super) fn hush(&mut self, ext: &str) {
        if self.sent_to(ext).is_some() {
            self.sent.retain(|(n, _)| n != ext);
            if let Some(worker) = self.workers.iter().find(|w| w.spec().name == ext) {
                worker.cancel();
            }
        }
    }

    /// The items of extension `ext`'s last answer to a query that `like` accepts, while the
    /// newest query sent is one too.
    fn answer(&self, ext: &str, like: &dyn Fn(&Request) -> bool) -> Option<&[ExtItem]> {
        self.sent_to(ext).filter(|r| like(r))?;
        let (_, request, items) = self.found.iter().find(|(n, ..)| n == ext)?;
        like(request).then_some(items.as_slice())
    }

    /// The items `filter: "ext"` view `view` of extension `ext` shows instead of its `open`
    /// answer's: the answer to its query, while one is asked.
    pub(super) fn found_in_view(&self, ext: &str, view: &str) -> Option<&[ExtItem]> {
        self.answer(ext, &|r| of_view(r, view))
    }

    /// The keyword rows of extension `ext` (root search), while a keyword query is asked.
    pub(super) fn found_by_keyword(&self, ext: &str) -> Option<&[ExtItem]> {
        self.answer(ext, &|r| matches!(r, Request::Keyword { .. }))
    }

    /// `query` typed in `filter: "ext"` view `view` of extension `ext`: asked, or the query
    /// stopped when it is blank.
    pub(super) fn typed(&mut self, ext: &str, view: &str, query: &str) {
        if query.trim().is_empty() {
            self.hush(ext);
        } else {
            self.ask(ext, Request::Query { view: view.to_string(), query: query.to_string() });
        }
    }

    /// The direct rows for `cx.query`: `<keyword> <text>`, with the keyword of an extension,
    /// asks it and shows its last answer for that keyword. The other keyword queries stop.
    pub(super) fn keyword_rows(&mut self, cx: &Cx) -> Vec<Item> {
        let typed = cx.query.split_once(' ').map(|(k, rest)| (k, rest.trim())).filter(|(_, rest)| !rest.is_empty());
        let owner = typed.and_then(|(k, rest)| {
            let spec = self.specs.iter().find(|s| s.keywords.iter().any(|w| w == k))?;
            Some((spec.name.clone(), k, rest))
        });
        let other = |(ext, r): &(String, Request)| {
            matches!(r, Request::Keyword { .. }) && owner.as_ref().is_none_or(|(o, ..)| o != ext)
        };
        for worker in self.workers.iter().filter(|w| self.sent.iter().any(|s| other(s) && s.0 == w.spec().name)) {
            worker.cancel();
        }
        self.sent.retain(|s| !other(s));
        let Some((ext, keyword, rest)) = owner else { return vec![] };
        self.ask(&ext, Request::Keyword { keyword: keyword.to_string(), query: rest.to_string() });
        let found = self.answer(&ext, &|r| of_keyword(r, keyword)).unwrap_or_default();
        found.iter().map(|e| items::item(&ext, e)).collect()
    }

    /// Extension `ext`'s answer to query `request`. True when the screen changed: it answers
    /// the newest query sent, or it failed for the view on the screen.
    pub(super) fn queried(&mut self, ext: &str, request: Request, result: Result<Reply, String>) -> bool {
        if self.sent_to(ext) != Some(&request) {
            return false;
        }
        let view = match &request {
            Request::Query { view, .. } => Some(format!("{ext}/{view}")),
            _ => None,
        };
        match result {
            Ok(Reply::Items(items)) => {
                self.error.take_if(|(v, _)| Some(&*v) == view.as_ref());
                self.found.retain(|(n, ..)| n != ext);
                self.found.push((ext.to_string(), request, items));
                true
            }
            Ok(_) => false,
            Err(e) => {
                (self.hooks.log)(&e);
                let shown = view.filter(|v| self.showing.as_ref() == Some(v) && (self.hooks.visible)());
                shown.map(|v| self.error = Some((v, e))).is_some()
            }
        }
    }
}

#[cfg(test)]
mod tests;
