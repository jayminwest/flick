//! The KOTA menu bar item: the presence glyph plus the pending-card count, the tooltip and
//! the menu (`view::menu`), kept in step with the presence on the main thread.
//!
//! The item shows only once the module has started, while the `[kota]` table sets a key
//! (the Mac that polls) and `status_item` is true; otherwise it is hidden, so a Mac
//! without `[kota]` has no KOTA item. It is redrawn only when its title, tooltip or rows
//! change. Menu rows: Ask KOTA… and Inbox route like the module's hotkeys `ask` and
//! `inbox` (`platform::status_item` opener); Open Dashboard and Refresh Now are picks.
//! Opening the menu starts a round (at most one per `io::REFRESH_EVERY` seconds); an open
//! menu takes the new rows in place.
//!
//! Picks and menu opens run while `AppKit` is mid-event, so `wire.rs` only queues their
//! key and posts `ModuleChanged`; `requests` handles them on the module's turn
//! (mulch mx-fcbc43). A change to down posts one notification when `notify_down` is true
//! (`notify::post` only: herdr owns the click handler, mx-444675).

use super::presence::State;
use super::view::{self, Entry};
use super::{Kota, io};

/// The queued key for "the menu is about to open".
pub const MENU_OPENED: &str = "menu-opened";
/// The Open Dashboard pick.
pub const DASH: &str = "dash";
/// The Refresh Now pick.
pub const REFRESH: &str = "refresh";
/// The Inbox row's hotkey key.
pub const INBOX: &str = "inbox";
/// The cards view the Inbox row opens: the `message` module's recent list, by name.
pub const INBOX_VIEW: (&str, &str) = ("message", "recent");

/// What the item does on the system; `wire::UI` is the real thing, tests use
/// `testkit::UI`.
#[derive(Clone, Copy)]
pub struct Ui {
    /// Show or update the item: title, tooltip, menu rows.
    pub show: fn(&str, &str, &[Entry]),
    /// Remove the item.
    pub hide: fn(),
    /// Queue `MENU_OPENED` each time the item's menu opens.
    pub listen: fn(),
    /// The keys queued since the last call (picks and `MENU_OPENED`).
    pub take: fn() -> Vec<String>,
    /// Post a notification: title, body.
    pub notify: fn(&str, &str),
    /// Open a URL in its default app.
    pub open_url: fn(&str),
}

/// What the item shows: title, tooltip, rows.
pub type Shown = (String, String, Vec<Entry>);

impl Kota {
    /// The item should be in the menu bar.
    fn wants_item(&self) -> bool {
        self.polling() && self.settings.status_item
    }

    /// Show, update or hide the item to match the presence and the pending count.
    pub(super) fn sync_item(&mut self) {
        if !self.wants_item() {
            if self.shown.take().is_some() {
                (self.ui.hide)();
            }
            return;
        }
        let now = (self.hooks.now)();
        let offset = (self.hooks.utc_offset)(i64::try_from(now).unwrap_or(0));
        let p = self.shared.lock();
        let next = (
            view::title(&p.presence, self.pending),
            view::tooltip(&p.presence, now),
            view::menu(&p.presence, self.pending, now, offset),
        );
        drop(p);
        if self.shown.as_ref() == Some(&next) {
            return;
        }
        if self.shown.is_none() {
            (self.ui.listen)();
        }
        (self.ui.show)(&next.0, &next.1, &next.2);
        self.shown = Some(next);
    }

    /// Handle the queued picks and menu opens, then notify on a change to down.
    pub(super) fn requests(&mut self) {
        let (shared, s, hooks) = (&self.shared, &self.settings, self.hooks);
        for key in (self.ui.take)() {
            match key.as_str() {
                DASH => (self.ui.open_url)(&s.dash),
                REFRESH | MENU_OPENED => drop(io::refresh(shared, s, hooks)),
                _ => {}
            }
        }
        let notice = {
            let mut p = shared.lock();
            let down = std::mem::take(&mut p.transitions).iter().any(|t| t.to == State::Down);
            (down && self.active && s.notify_down).then(|| view::down_notice(&p.presence, &s.machine))
        };
        if let Some((title, body)) = notice {
            (self.ui.notify)(&title, &body);
        }
    }
}

#[cfg(test)]
mod tests;
