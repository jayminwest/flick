//! The count of cards that wait on the user, and of unread posts (`seen.rs`), sent to every
//! module as `Event::CardsPending { count, unread }` (plan flick-4354: the KOTA status
//! item's badge). This module owns the cards, so it decides what counts; nothing else reads
//! its table.
//!
//! A stored card (the newest `max_history` messages) waits on the user when all hold:
//! - `Card::waits_on_user`: it is `open` and has at least one enabled action;
//! - its press state is not busy: no send to KOTA, wait for KOTA's update, or local run is
//!   in flight (`dispatch::Ui::busy`). An error line or a shell confirm step leaves it
//!   waiting on the user;
//! - the user has not dismissed it (x, Esc, a click, `card dismiss`, a `dismiss` action).
//!   A card that only timed out is out of the corner but still waits: that is what the
//!   badge is for.
//!
//! Dismissals are stored (`seen.rs`, flick-07cd): a restart does not bring them back.
//!
//! `announce` runs at `Started` (the first counts), after every `ModuleChanged` drain,
//! after every verb and when the message list opens, and sends the event only when a count
//! changed. At `Reloaded` it
//! sends the count even when unchanged: a module the reload enabled (`kota`) got `Started`
//! but has not heard it yet (flick-b220). `message` never learns who started; the core
//! tells every module that a reload ended, so neither imports the other.

use super::seen::{Dismissal, Seen};
use super::store::Messages;
use super::{Inbox, card};
use crate::core::Cx;

impl Inbox {
    /// Cards that wait on the user now.
    pub(super) fn waiting(&self, cx: &Cx) -> u32 {
        let waits = |id: &str| !self.ui.get(id).is_some_and(super::dispatch::Ui::busy);
        let n = cx
            .store
            .cards(self.settings.max_history)
            .iter()
            .filter(|m| m.dismissed != Some(Dismissal::User) && waits(&m.id))
            .filter(|m| card::stored(m).is_some_and(|c| c.waits_on_user()))
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    /// Send `Event::CardsPending` if a count changed since the last one sent.
    pub(super) fn announce(&mut self, cx: &Cx) {
        let counts = (self.waiting(cx), cx.store.unread_count());
        if self.announced != Some(counts) {
            self.announced = Some(counts);
            (self.env.pending)(counts.0, counts.1);
        }
    }
}
