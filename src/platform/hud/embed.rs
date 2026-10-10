//! Cards drawn into another window's view (a surface transcript row) by the same renderer as
//! the corner cards. The host keeps the `Embedded` (its view and live controls), places
//! `view()` and redraws by drawing again with the old one, which carries what the user typed
//! or picked. Presses on embedded cards go to the one handler set with `on_press`, with the
//! `key()` of the card's view; the host then asks `press` for the action and the values.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::{MainThreadOnly, msg_send};
use objc2_app_kit::NSView;
use objc2_foundation::NSRect;

use super::card_layout::{CardUi, carry};
use super::card_view;
use super::controls::Controls;
use super::view::ContentView;
use crate::core::card::Card;
use crate::core::card::action::values_json;
use crate::platform::mtm;

/// Where presses on embedded cards go: the card view's `key()`, the button's tag.
type Handler = fn(usize, isize);

thread_local! {
    static ON_PRESS: Cell<Option<Handler>> = const { Cell::new(None) };
}

/// Send presses on embedded cards to `handler` (the pressed card's `key()`, the button's
/// tag), on the main thread after `AppKit` has delivered the click, outside any HUD state
/// borrow. The latest handler wins.
pub fn on_press(handler: Handler) {
    ON_PRESS.set(Some(handler));
}

/// A press the HUD's stack did not own.
pub(super) fn pressed(host: usize, tag: isize) {
    if let Some(handler) = ON_PRESS.get() {
        handler(host, tag);
    }
}

/// One drawn card: its flipped view, `height()` tall at the width it was drawn at.
pub struct Embedded {
    view: Retained<ContentView>,
    controls: Controls,
    height: f64,
    width: f64,
}

impl Embedded {
    /// Draw `card` in state `ui` at `width`. With `before` (the same card's last drawing), its
    /// inputs keep what the user entered unless the card changed their initial values.
    pub fn draw(card: &Card, ui: &CardUi, width: f64, before: Option<&Embedded>) -> Embedded {
        let mtm = mtm();
        let old = before.map(|b| (b.controls.initial.clone(), b.controls.values()));
        let old = old.as_ref().map(|(i, l)| (i.as_slice(), l.as_slice()));
        let values = carry(old, card.inputs());
        // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
        let view: Retained<ContentView> =
            unsafe { msg_send![ContentView::alloc(mtm), initWithFrame: NSRect::ZERO] };
        let (height, controls) = card_view::render(mtm, &view, card, ui, &values, width);
        view.setFrameSize(objc2_foundation::NSSize::new(width, height));
        Embedded { view, controls, height, width }
    }

    pub fn view(&self) -> &NSView {
        &self.view
    }

    pub fn height(&self) -> f64 {
        self.height
    }

    pub fn width(&self) -> f64 {
        self.width
    }

    /// The view's address, as `on_press` reports it.
    pub fn key(&self) -> usize {
        Retained::as_ptr(&self.view).cast::<()>() as usize
    }

    /// What the button tagged `tag` presses: the action id (or `CANCEL`) and the values JSON,
    /// or the error to show when the values are over the cap.
    pub fn press(&self, tag: isize) -> Option<(String, Result<String, String>)> {
        let action = self.controls.action(tag)?;
        Some((action, values_json(&self.controls.values())))
    }
}
