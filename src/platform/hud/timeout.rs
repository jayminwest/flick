//! Card timeouts that wait for the user (flick-cb7d). A card's `Options::timeout_secs` runs
//! only while someone is at the Mac: when it ends with no keyboard or mouse input since it
//! was set (the user is away, the screen locked or the display asleep), or the timer fired
//! late (the Mac slept), the card stays and is rechecked every `AWAY_RECHECK` seconds; the
//! first input then starts its full timeout again. A pointer resting on a card holds it off
//! too (`HOVER_RECHECK`). `next` is the pure decision; `arm` and `expire` run it on timers.

use std::time::{SystemTime, UNIX_EPOCH};

use objc2_app_kit::NSEvent;

use super::{Dismissed, STATE, close, contains};
use crate::platform::{events, timer};

/// Seconds the pointer over a card holds off its timeout, rechecked each time.
const HOVER_RECHECK: f64 = 2.0;
/// Seconds between checks for input while a card waits for the user to come back.
const AWAY_RECHECK: f64 = 2.0;
/// How late a timer may fire and still count as on time; later means the Mac slept.
const LATE: f64 = 5.0;

/// What a card's timer does when it fires.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Next {
    Close,
    /// The pointer rests on it: check again in `HOVER_RECHECK`.
    Hover,
    /// Nobody was there: check again in `AWAY_RECHECK`, as away.
    Away,
    /// The user is back after `Away`: the full timeout again.
    Restart,
}

/// The decision when a timer set `due` seconds ago (by its own reckoning) fires `since`
/// wall-clock seconds after it was set, `idle` seconds after the last input; `away` when it
/// was an `Away` recheck.
pub fn next(hovered: bool, idle: f64, since: f64, due: f64, away: bool) -> Next {
    let present = idle < since && since < due + LATE;
    match (hovered, present, away) {
        (true, _, _) => Next::Hover,
        (false, false, _) => Next::Away,
        (false, true, true) => Next::Restart,
        (false, true, false) => Next::Close,
    }
}

fn wall() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Start card `id`'s timeout for the show or update numbered `epoch`; 0 seconds: none.
pub(super) fn arm(id: &str, epoch: u64, secs: f64) {
    if secs > 0.0 {
        after(id, epoch, secs, false);
    }
}

fn after(id: &str, epoch: u64, due: f64, away: bool) {
    let (id, set) = (id.to_string(), wall());
    timer::after(due, move || expire(&id, epoch, set, due, away));
}

/// A timer of card `id`: dismiss it unless a later show or update re-armed it, or `next`
/// keeps it.
fn expire(id: &str, epoch: u64, set: f64, due: f64, away: bool) {
    let live = STATE.with_borrow(|s| {
        let e = s.cards.iter().find(|e| e.id == id && e.epoch == epoch)?;
        let hovered =
            e.views.panel.isVisible() && contains(e.views.panel.frame(), NSEvent::mouseLocation());
        Some((hovered, e.opts.timeout_secs))
    });
    let Some((hovered, secs)) = live else { return };
    match next(hovered, events::idle_secs(), wall() - set, due, away) {
        Next::Close => close(id, Dismissed::Timeout),
        Next::Hover => after(id, epoch, HOVER_RECHECK, false),
        Next::Away => after(id, epoch, AWAY_RECHECK, true),
        Next::Restart => after(id, epoch, secs, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_closes_only_after_the_user_was_there() {
        // Input during its 20 s, on time: it closes.
        assert_eq!(next(false, 3.0, 20.0, 20.0, false), Next::Close);
        // No input since it showed: it waits.
        assert_eq!(next(false, 25.0, 20.0, 20.0, false), Next::Away);
        assert_eq!(next(false, 20.0, 20.0, 20.0, false), Next::Away);
        // The Mac slept: the timer fired hours late, after the wake keypress. It waits.
        assert_eq!(next(false, 1.0, 8.0 * 3600.0, 20.0, false), Next::Away);
        // Away rechecks: still nobody, then input: the full timeout again.
        assert_eq!(next(false, 40.0, 2.0, 2.0, true), Next::Away);
        assert_eq!(next(false, 0.5, 2.0, 2.0, true), Next::Restart);
        // The pointer on it holds it off, away or not.
        assert_eq!(next(true, 0.0, 20.0, 20.0, false), Next::Hover);
        assert_eq!(next(true, 90.0, 2.0, 2.0, true), Next::Hover);
        // Back from a hover recheck with input: it closes.
        assert_eq!(next(false, 0.1, 2.0, 2.0, false), Next::Close);
    }
}
