//! Keyboard reach for cards (plan flick-7da1 step 8): `focus_top` makes the newest card's
//! panel key, `unfocus` gives the keyboard back. What the keys do is the pure `keys`; this
//! file applies it to the panels.
//!
//! Card panels are non-activating (`NSWindowStyleMask::NonactivatingPanel`), so making one
//! key never activates Flick: the front app stays the active app (its menu bar stays) and
//! only the key window moves to the card, as with the launcher panel. Giving the keyboard
//! back ends any field edit, then orders the panel out and straight back in within the same
//! turn of the run loop (nothing redraws in between). A key window of an app that is not
//! active leaving the screen hands key status back to the active app's window, the same
//! path the launcher takes when it hides; ordering it back in (`orderFrontRegardless`) does
//! not make it key. `NSWindow::resignKeyWindow` is not used: `AppKit` forbids calling it.
//!
//! The keyboard goes back on Esc, after any press of a card's button (a click after typing
//! in a field, Space on a focused button, ⌘1..⌘6, ⌘↵), and when the card closes (ordering a
//! key panel out hands the keyboard back the same way).

use objc2::rc::Retained;
use objc2_app_kit::{NSControl, NSEvent, NSEventModifierFlags, NSEventType, NSPanel, NSResponder};

use super::keys::{self, Command, Mods};
use super::{STATE, State};
use crate::platform::timer;

/// Make the newest card key, its keyboard on its first enabled control, without activating
/// Flick. Returns whether there was a card.
pub fn focus_top() -> bool {
    let top = STATE.with_borrow(|s| s.cards.first().map(target));
    top.is_some_and(|(panel, stops)| {
        take(&panel, &stops);
        true
    })
}

/// Give the keyboard back to the front app if a card holds it; the card stays. Returns
/// whether one did.
pub fn unfocus() -> bool {
    let key = STATE.with_borrow(|s| key_panel(s, None));
    key.is_some_and(|panel| {
        give_back(&panel);
        true
    })
}

/// After a press on the card in panel `window`: give the keyboard back if it holds it.
pub(super) fn pressed(window: usize) {
    if let Some(panel) = STATE.with_borrow(|s| key_panel(s, Some(window))) {
        give_back(&panel);
    }
}

/// The key card panel (only the one at `window`, if given).
fn key_panel(s: &State, window: Option<usize>) -> Option<Retained<NSPanel>> {
    let mut views = s.cards.iter().map(|e| &e.views);
    let key = views.find(|v| v.panel.isKeyWindow() && window.is_none_or(|w| v.key() == w));
    key.map(|v| v.panel.clone())
}

/// A card's panel and its Tab stops.
fn target(e: &super::Entry) -> (Retained<NSPanel>, Vec<Retained<NSControl>>) {
    (
        e.views.panel.clone(),
        e.controls.as_ref().map(super::controls::Controls::stops).unwrap_or_default(),
    )
}

fn responder(c: &NSControl) -> &NSResponder {
    c
}

/// Make `panel` key with the keyboard on its first enabled stop (else on the panel).
fn take(panel: &NSPanel, stops: &[Retained<NSControl>]) {
    panel.makeKeyAndOrderFront(None);
    let first = stops.iter().find(|c| c.isEnabled());
    panel.makeFirstResponder(first.map(|c| responder(c)));
}

fn give_back(panel: &NSPanel) {
    panel.makeFirstResponder(None);
    if panel.isVisible() {
        panel.orderOut(None);
        panel.orderFrontRegardless();
    }
}

/// The modifiers of `flags` that `keys` looks at.
fn mods(flags: NSEventModifierFlags) -> Mods {
    Mods {
        command: flags.contains(NSEventModifierFlags::Command),
        shift: flags.contains(NSEventModifierFlags::Shift),
        option: flags.contains(NSEventModifierFlags::Option),
        control: flags.contains(NSEventModifierFlags::Control),
    }
}

/// A key-down delivered to the key card panel at `window`: whether it was a card command,
/// which then runs on the next turn of the run loop (it may reorder the panel the event is
/// still being delivered to). Other events go on to `AppKit`.
pub(super) fn key_event(window: usize, is_key: bool, event: &NSEvent) -> bool {
    if !is_key || event.r#type() != NSEventType::KeyDown {
        return false;
    }
    let chars = event.charactersIgnoringModifiers().map(|s| s.to_string());
    let key = keys::key(event.keyCode(), chars.as_deref().unwrap_or(""));
    let Some(command) = keys::command(key, mods(event.modifierFlags())) else { return false };
    timer::after(0.0, move || run(window, command));
    true
}

/// Run `command` on the card in panel `window`, if it still holds the keyboard.
fn run(window: usize, command: Command) {
    let found = STATE.with_borrow(|s| {
        let i =
            s.cards.iter().position(|e| e.views.key() == window && e.views.panel.isKeyWindow())?;
        let e = &s.cards[i];
        let controls = e.controls.as_ref();
        let pick = match command {
            Command::Unfocus => Pick::GiveBack,
            Command::Cycle { back } => {
                let stops: Vec<_> = target(e).1.into_iter().filter(|c| c.isEnabled()).collect();
                let current = stops.iter().position(|c| holds(&e.views.panel, c));
                keys::cycle(current, stops.len(), back)
                    .map_or(Pick::Nothing, |n| Pick::Focus(stops[n].clone()))
            }
            // The confirm step's Cancel and Run are not numbered actions.
            Command::Press(n) => controls
                .filter(|c| c.confirm.is_none())
                .and_then(|c| c.buttons.get(n).cloned())
                .map_or(Pick::Nothing, Pick::Click),
            Command::Primary => controls
                .and_then(|c| {
                    let id = &c.card.primary()?.id;
                    let n = c.actions.iter().position(|a| a == id)?;
                    c.buttons.get(n).cloned()
                })
                .map_or(Pick::Nothing, Pick::Click),
            Command::Move { up } => {
                let visible = s.cards.iter().take_while(|e| e.views.panel.isVisible()).count();
                let top = s.corner.is_none_or(super::Corner::top);
                keys::moved(i, visible, up, top)
                    .map_or(Pick::Nothing, |n| Pick::Card(target(&s.cards[n])))
            }
        };
        Some((e.views.panel.clone(), pick))
    });
    let Some((panel, pick)) = found else { return };
    match pick {
        Pick::Nothing => {}
        Pick::GiveBack => give_back(&panel),
        Pick::Focus(c) => {
            panel.makeFirstResponder(Some(responder(&c)));
        }
        // A disabled button does nothing; an enabled one goes through its target as a
        // click does (`pressed`), which then gives the keyboard back.
        Pick::Click(b) => {
            if b.isEnabled() {
                // SAFETY: nil is a valid sender; the button's target outlives it (`target`).
                unsafe { b.performClick(None) };
            }
        }
        Pick::Card((other, stops)) => take(&other, &stops),
    }
}

/// What `run` does once the state borrow is over.
enum Pick {
    Nothing,
    GiveBack,
    Focus(Retained<NSControl>),
    Click(Retained<super::controls::Button>),
    Card((Retained<NSPanel>, Vec<Retained<NSControl>>)),
}

/// Whether control `c` holds the keyboard in `panel`: it is the first responder, or a text
/// field being edited (its field editor is).
fn holds(panel: &NSPanel, c: &NSControl) -> bool {
    let first = panel.firstResponder();
    let is_first = first.as_deref().is_some_and(|r| std::ptr::eq(r, responder(c)));
    is_first || c.currentEditor().is_some()
}
