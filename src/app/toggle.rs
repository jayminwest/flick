//! Hotkey views: a press shows a view or hides the launcher, and a held hotkey
//! (`ListView::cycle`, e.g. ⌘Tab) steps through its view until its modifiers come up.
//! Holding its key steps again at the system key repeat rate, back while ⇧ is down.

use std::cell::Cell;
use std::time::Instant;

use super::Screen;
use super::{State, with_state};
use crate::control;
use crate::core::{Cycle, Event, ListView};
use crate::platform::{hotkeys, keytap, timer};
use crate::ui;

/// ⌘, ⌥ and ⌃ (`CGEventFlags`). ⇧ only picks the direction, so releasing it does not count.
const HELD: u64 = 0x10_0000 | 0x8_0000 | 0x4_0000;
const SHIFT: u64 = 0x2_0000;
/// How often a cycle checks whether its modifiers are still down.
const POLL_SECS: f64 = 0.02;

thread_local! {
    /// A cycle is waiting for its modifiers to come up.
    static WATCHING: Cell<bool> = const { Cell::new(false) };
    /// When the cycle's key went down, and when it last stepped the selection.
    static PRESSED: Cell<Option<(Instant, Instant)>> = const { Cell::new(None) };
}

/// Show `request` (root search for `None`), or hide the launcher if it already shows it. A
/// cycle request on its own view steps the selection instead of hiding.
pub fn toggle_view(request: Option<ListView>) {
    let visible = ui::is_visible();
    let cycle = request.as_ref().and_then(|r| r.cycle);
    let showing = visible && with_state(|s| s.screen.shows(request.as_ref())).unwrap_or(false);
    if showing && cycle.is_none() {
        ui::hide();
        return;
    }
    if !showing {
        with_state(|s| {
            if request.is_none() {
                s.registry.dispatch(Event::LauncherOpened, &mut s.env.cx(""));
                control::publish(Event::LauncherOpened);
            }
            s.status = None;
            s.enter(request);
        });
        if !visible {
            ui::show();
        }
    }
    if let Some(cycle) = cycle {
        with_state(|s| s.step(cycle, !showing));
        let now = Instant::now();
        PRESSED.set(Some((now, now)));
        watch();
    }
}

impl State {
    /// Select the item `cycle` moves to; `first`: the press that opened the view.
    fn step(&mut self, cycle: Cycle, first: bool) {
        self.status = None;
        let to = cycle.select(self.selected, self.results.len(), first);
        self.move_selection(to as isize - self.selected as isize);
    }
}

/// Poll the modifiers until they come up, once per cycle however many presses it takes.
fn watch() {
    if !WATCHING.replace(true) {
        timer::after(POLL_SECS, poll);
    }
}

/// Released: activate the selected item of the view on the panel. A hidden launcher (Escape,
/// a click elsewhere) ends the cycle without one.
fn poll() {
    if !ui::is_visible() {
        WATCHING.set(false);
    } else if keytap::current_flags() & HELD != 0 {
        repeat();
        timer::after(POLL_SECS, poll);
    } else {
        WATCHING.set(false);
        with_state(|s| {
            if matches!(s.screen, Screen::List(_)) {
                s.activate();
            }
        });
    }
}

/// Step again while the cycle's key stays down, as key repeat would.
fn repeat() {
    let Some((down, last)) = PRESSED.get() else { return };
    if !hotkeys::key_down() {
        PRESSED.set(None);
        return;
    }
    let now = Instant::now();
    let held = |since: Instant| now.duration_since(since).as_secs_f64();
    if repeats(held(down), held(last), hotkeys::key_repeat()) {
        let back = keytap::current_flags() & SHIFT != 0;
        with_state(|s| s.step(if back { Cycle::Previous } else { Cycle::Next }, false));
        PRESSED.set(Some((down, now)));
    }
}

/// Whether a key down for `held` seconds, `since_step` after its last step, repeats now under
/// `(delay, interval)`.
fn repeats(held: f64, since_step: f64, (delay, interval): (f64, f64)) -> bool {
    held >= delay && since_step >= interval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_key_repeats_after_the_delay_at_the_interval() {
        let timing = (0.3, 0.05);
        assert!(!repeats(0.2, 0.2, timing));
        assert!(repeats(0.3, 0.3, timing));
        assert!(!repeats(0.32, 0.02, timing));
        assert!(repeats(0.36, 0.06, timing));
    }
}
