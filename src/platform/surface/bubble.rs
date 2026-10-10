//! A bubble row in a normal (not private) surface, built so its text can be selected and
//! copied (flick-0955):
//!
//! - The row is not window background. The panel is movable by its background, and `AppKit`
//!   hands the window server every area whose view says `mouseDownCanMoveWindow`; a drag
//!   that starts there moves the window instead of reaching Flick. That was the bubble's
//!   padding and every gap around its text, so a selection started just left of the first
//!   character (or anywhere off a glyph) dragged the whole window.
//! - A click or drag on the bubble's fill goes to its text view, which selects from the
//!   nearest character, as in Messages.
//! - The text view takes the first click even while the panel is not key (after the user
//!   worked in another app while KOTA answered); otherwise that click only makes it key.
//! - Its context menu starts with Copy Message: the whole message (`rows::message_text`)
//!   to the pasteboard, selection or not.

use std::cell::{OnceCell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{NSEvent, NSMenu, NSMenuItem, NSResponder, NSText, NSTextView, NSView};
use objc2_foundation::{NSAttributedString, NSObject, NSPoint, NSRect, NSString};

use super::{render, rows};
use crate::platform::pasteboard;

/// What Copy Message copies: the bubble's markdown-lite source.
pub(super) struct Source {
    md: RefCell<String>,
}

define_class!(
    // A bubble's selectable text.
    #[unsafe(super(NSTextView, NSText, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickBubbleText"]
    #[ivars = Source]
    pub(super) struct BubbleText;

    impl BubbleText {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        // The text view's own menu (Copy, Look Up, …) under Copy Message.
        #[unsafe(method_id(menuForEvent:))]
        fn menu_for_event(&self, event: &NSEvent) -> Option<Retained<NSMenu>> {
            // SAFETY: the superclass method, with the argument it was called with.
            let menu: Option<Retained<NSMenu>> =
                unsafe { msg_send![super(self), menuForEvent: event] };
            let mtm = self.mtm();
            let menu = menu.unwrap_or_else(|| NSMenu::new(mtm));
            menu.insertItem_atIndex(&NSMenuItem::separatorItem(mtm), 0);
            menu.insertItem_atIndex(&copy_item(mtm, self), 0);
            Some(menu)
        }

        #[unsafe(method(copyMessage:))]
        fn copy_message(&self, _sender: Option<&AnyObject>) {
            pasteboard::set_text(rows::message_text(&self.ivars().md.borrow()));
        }
    }
);

/// The Copy Message item, sending `copyMessage:` to `target`.
fn copy_item(mtm: MainThreadMarker, target: &BubbleText) -> Retained<NSMenuItem> {
    // SAFETY: `copyMessage:` is a method of `BubbleText` that takes the sending item.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Copy Message"),
            Some(sel!(copyMessage:)),
            &NSString::from_str(""),
        )
    };
    // SAFETY: the item holds its target weakly; the text view outlives the menu it opened
    // (it stays in its window while the menu shows).
    unsafe { item.setTarget(Some(target)) };
    item
}

/// A selectable text view showing `string`, whose Copy Message copies `md`.
pub(super) fn text_view(
    mtm: MainThreadMarker,
    string: &NSAttributedString,
    md: &str,
) -> Retained<NSTextView> {
    let this = BubbleText::alloc(mtm).set_ivars(Source { md: RefCell::new(md.to_string()) });
    // SAFETY: NSTextView's `initWithFrame:` (it builds its own text system); +1 object.
    let view: Retained<BubbleText> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
    render::show_in(&view, string, true);
    view.into_super()
}

/// The bubble's fill and text, set once its parts are built.
#[derive(Default)]
pub(super) struct Parts {
    back: OnceCell<Retained<NSView>>,
    text: OnceCell<Retained<NSView>>,
}

define_class!(
    // A bubble's row: flipped, never window background, its fill a handle on its text.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickBubbleRow"]
    #[ivars = Parts]
    pub(super) struct BubbleRow;

    impl BubbleRow {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        // A point on the fill (around the text) goes to the text. Tail expression only
        // (mx-43d3f4).
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            // SAFETY: the superclass method, with the argument it was called with.
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            let parts = self.ivars();
            match (hit, parts.back.get(), parts.text.get()) {
                (Some(h), Some(back), Some(text)) if h.isDescendantOf(back) => Some(text.clone()),
                (h, ..) => h,
            }
        }
    }
);

/// An empty bubble row; `hold` its fill and text once they are its subviews.
pub(super) fn row(mtm: MainThreadMarker) -> Retained<BubbleRow> {
    let this = BubbleRow::alloc(mtm).set_ivars(Parts::default());
    // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
    unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
}

impl BubbleRow {
    pub(super) fn hold(&self, back: &NSView, text: &NSView) {
        let parts = self.ivars();
        let _ = parts.back.set(back.retain());
        let _ = parts.text.set(text.retain());
    }
}
