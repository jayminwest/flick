//! The count of cards that wait on the user, sent to every module as
//! `Event::CardsPending { count }` (plan flick-4354: the KOTA status item's badge). This
//! module owns the cards, so it decides what counts; nothing else reads its table.
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
//! Dismissals live in memory, so after a restart every stored open card with actions
//! counts again until the user acts on or dismisses it.
//!
//! `announce` runs at `Started` (the first count), after every `ModuleChanged` drain and
//! after every verb, and sends the event only when the count changed. At `Reloaded` it
//! sends the count even when unchanged: a module the reload enabled (`kota`) got `Started`
//! but has not heard it yet (flick-b220). `message` never learns who started; the core
//! tells every module that a reload ended, so neither imports the other.

use super::store::Messages;
use super::{Inbox, card};
use crate::core::Cx;

impl Inbox {
    /// Cards that wait on the user now.
    pub(super) fn waiting(&self, cx: &Cx) -> u32 {
        let waits = |id: &str| {
            !self.ui.get(id).is_some_and(super::dispatch::Ui::busy)
                && (!self.dismissed.contains(id) || self.expired.contains(id))
        };
        let n = cx
            .store
            .cards(self.settings.max_history)
            .iter()
            .filter(|m| waits(&m.id) && card::stored(m).is_some_and(|c| c.waits_on_user()))
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    /// Send `Event::CardsPending` if the count changed since the last one sent.
    pub(super) fn announce(&mut self, cx: &Cx) {
        let n = self.waiting(cx);
        if self.announced != Some(n) {
            self.announced = Some(n);
            (self.env.pending)(n);
        }
    }
}
