//! Clicks and presses on a card panel: the close button, a click on a text card, and an
//! action button press (or one on an `embed` card), and a click on the `+N more` pill.

use super::{
    CardUi, Dismissed, Draw, ON_PRESS, STATE, arm, close, embed, fill, focus, next_epoch, relayout,
    stack,
};
use crate::core::card::action::values_json;
use crate::platform::workspace;

/// A click on the card in panel `window`: on its close button, or elsewhere.
pub(super) fn clicked(window: usize, on_close: bool) {
    let hit = STATE.with_borrow(|s| {
        let e = s.cards.iter().find(|e| e.views.key() == window)?;
        Some((e.id.clone(), e.link.clone(), e.click_dismisses))
    });
    match hit {
        Some((id, _, _)) if on_close => close(&id, Dismissed::Closed),
        Some((id, link, true)) => {
            close(&id, Dismissed::Clicked);
            if let Some(link) = link {
                workspace::open_url(&link);
            }
        }
        _ => {}
    }
}

/// A press of the button tagged `tag` on the card in panel `window` (else on an `embed` card
/// drawn into view `host`): hand the action and the values to the `on_press` handler, or
/// redraw the card with the error if the values are over the cap.
pub(super) fn pressed(window: usize, host: usize, tag: isize) {
    let hit = STATE.with_borrow(|s| {
        let e = s.cards.iter().find(|e| e.views.key() == window)?;
        let c = e.controls.as_ref()?;
        Some((e.id.clone(), c.action(tag)?, values_json(&c.values())))
    });
    if hit.is_none() {
        embed::pressed(host, tag);
    }
    match hit {
        Some((id, action, Ok(values))) => {
            if let Some(handler) = ON_PRESS.get() {
                handler(&id, &action, values);
            }
            focus::pressed(window);
        }
        Some((id, _, Err(why))) => refill_with_error(&id, &why),
        None => {}
    }
}

/// Redraw card `id` as it was drawn, with `error` as its error line.
fn refill_with_error(id: &str, error: &str) {
    let mtm = crate::platform::mtm();
    STATE.with_borrow_mut(|s| {
        let Some(e) = s.cards.iter_mut().find(|e| e.id == id) else { return };
        let Some(c) = &e.controls else { return };
        let (card, pending, confirm) = (c.card.clone(), c.pending, c.confirm.clone());
        let ui = CardUi { pending, error: Some(error), confirm: confirm.as_deref(), note: None };
        let (width, opts, epoch) = (e.size.0, e.opts, e.epoch);
        fill(mtm, e, Draw::Card(&card, &ui), width, opts, epoch);
        relayout(s, mtm);
    });
}

/// A click on the `+N more` pill: page to the hidden cards (`stack::cycle`). Every card that
/// shows after it starts its full timeout again, so a card does not appear only to vanish.
pub(super) fn more_clicked() {
    let mtm = crate::platform::mtm();
    let shown: Vec<(String, u64, f64)> = STATE.with_borrow_mut(|s| {
        let n = s.shown;
        stack::cycle(&mut s.cards, n);
        relayout(s, mtm);
        let n = s.shown;
        s.cards
            .iter_mut()
            .take(n)
            .map(|e| {
                e.epoch = next_epoch();
                (e.id.clone(), e.epoch, e.opts.timeout_secs)
            })
            .collect()
    });
    for (id, epoch, secs) in shown {
        arm(&id, epoch, secs);
    }
}
