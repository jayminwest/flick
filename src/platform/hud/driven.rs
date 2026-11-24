//! A driven instance's reach into the card stack (`crate::drive`): the cards as text, and a
//! press on a card's button that takes the same path as a click on it.

use std::fmt::Write as _;

use super::{STATE, clicks, focus};

/// The cards, newest first: `card: <id>` (`(keyboard)` when it holds the keyboard), then
/// its outline (`outline`) indented. Empty with no cards.
pub fn scene() -> String {
    STATE.with_borrow(|s| {
        let mut out = String::new();
        for e in &s.cards {
            let key = if focus::holds_key(&e.views) { " (keyboard)" } else { "" };
            let _ = writeln!(out, "card: {}{key}", e.id);
            for line in &e.outline {
                let _ = writeln!(out, "  {line}");
            }
        }
        out
    })
}

/// Press the button of `action` on card `id` as a click would: the values of its inputs go
/// to the `on_press` handler (or the card shows that they are over the cap), and a card that
/// holds the keyboard gives it back. In a shell confirm the buttons are the confirmed action
/// (Run) and `CANCEL`.
pub fn press(id: &str, action: &str) -> Result<(), String> {
    let (window, tag) = STATE.with_borrow(|s| {
        let e = s.cards.iter().find(|e| e.id == id).ok_or_else(|| format!("no card \"{id}\""))?;
        let c = e.controls.as_ref().ok_or_else(|| format!("card \"{id}\" has no actions"))?;
        let button = c.buttons.iter().find(|b| c.action(b.tag()).as_deref() == Some(action));
        let button = button.ok_or_else(|| format!("card \"{id}\" has no action \"{action}\""))?;
        if !button.isEnabled() {
            return Err(format!("action \"{action}\" on card \"{id}\" is off"));
        }
        Ok((e.views.key(), button.tag()))
    })?;
    clicks::pressed(window, 0, tag);
    Ok(())
}
