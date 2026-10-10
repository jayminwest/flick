//! The `AppKit` side of one card: a borderless, non-activating `NSPanel` (`CardPanel`) whose
//! flipped root view takes clicks without the panel becoming key, a blurred background, a
//! content view that a renderer fills (`text`, `card_view`), and an x close button. Also the
//! `+N more` pill.
//!
//! A card panel becomes key only when a click lands on a view that needs it (a text field:
//! `becomesKeyOnlyIfNeeded`) or `hud::focus_top` asks, never on show or on a button, and
//! being non-activating it never activates Flick. While key, it routes the card keys
//! (`focus`, `keys`) and the edit keys itself (Flick has no Edit menu).

use objc2::rc::Retained;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSColor, NSEvent, NSFont, NSImage, NSImageScaling, NSImageView,
    NSLineBreakMode, NSPanel, NSResponder, NSTextAlignment, NSTextField, NSTextView, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::platform::{edit, timer};

const LEVEL: isize = 25;
/// The close button's side, and the square in the top right corner that it answers to.
const CLOSE: f64 = 14.0;
const CLOSE_HIT: f64 = 30.0;
/// Inset of the close button from the card's top and right edges.
pub const CLOSE_INSET: f64 = 10.0;
/// Room a renderer leaves at the top right for the close button.
pub const CLOSE_ROOM: f64 = CLOSE_INSET + CLOSE + 6.0;

define_class!(
    // Borderless windows refuse key status by default. With `becomesKeyOnlyIfNeeded`, the
    // panel still becomes key only for a click on a view that needs it (a text field).
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickHudPanel"]
    pub struct CardPanel;

    impl CardPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        // The card keys (`keys`) while the panel is key, before the field editor or a
        // button sees them: Esc gives the keyboard back instead of cancelling an edit.
        #[unsafe(method(sendEvent:))]
        fn send_event(&self, event: &NSEvent) {
            if !super::focus::key_event(self.key(), self.isKeyWindow(), event) {
                // SAFETY: the superclass method, with the argument it was called with.
                unsafe { msg_send![super(self), sendEvent: event] }
            }
        }

        // The card keys may arrive here first, as key equivalents (⌘1..⌘6, ⌘↵). Flick has no
        // main menu, so the edit keys (⌘X, ⌘C, ⌘V, ⌘A, ⌘Z, ⇧⌘Z) go to the first responder from
        // here, as in the launcher panel (mx-43d850). Tail expression only (mx-43d3f4).
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            super::focus::key_event(self.key(), self.isKeyWindow(), event)
                || edit::send(event, self)
                // SAFETY: the superclass method, with the argument it was called with.
                || unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }
    }
);

impl CardPanel {
    /// The panel as an address, as `Views::key` gives it.
    fn key(&self) -> usize {
        std::ptr::from_ref(self).cast::<()>() as usize
    }
}

/// Whether a click on `v` goes to `v` rather than to the card: buttons (push, radio,
/// checkbox, pop-up), editable text fields and the field editor.
fn takes_clicks(v: &NSView) -> bool {
    v.isKindOfClass(NSButton::class())
        || v.isKindOfClass(NSTextView::class())
        || v.downcast_ref::<NSTextField>().is_some_and(NSTextField::isEditable)
}

define_class!(
    // The root view: flipped, so layouts read top-down. It takes the click itself unless it
    // lands on a control (`takes_clicks`), so a plain card is one click target.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickHudView"]
    pub struct HudView;

    impl HudView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        // A click lands on the card even though its window is never key.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            // SAFETY: NSView's own hitTest:, with its signature.
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            let me = std::ptr::from_ref(self).cast::<NSView>().cast_mut();
            // The hit view lives in this view's tree, which keeps it alive past the return.
            match hit {
                Some(v) if takes_clicks(&v) => Retained::as_ptr(&v).cast_mut(),
                _ => me,
            }
        }

        // Defer the work: it may close the panel this event is still being delivered to.
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let close = p.x >= self.bounds().size.width - CLOSE_HIT && p.y <= CLOSE_HIT;
            let window = self.window().map(|w| Retained::as_ptr(&w).cast::<()>() as usize);
            if let Some(window) = window {
                timer::after(0.0, move || super::clicked(window, close));
            }
        }
    }
);

define_class!(
    // Where a renderer puts the card's content: flipped, with the default hit testing.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickHudContent"]
    pub struct ContentView;

    impl ContentView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

pub fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

pub fn label(mtm: MainThreadMarker, font: &NSFont, color: &NSColor) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(""), mtm);
    l.setFont(Some(font));
    l.setTextColor(Some(color));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

/// The views of one card.
pub struct Views {
    pub panel: Retained<NSPanel>,
    root: Retained<HudView>,
    effect: Retained<NSVisualEffectView>,
    pub content: Retained<ContentView>,
    close: Retained<NSImageView>,
}

impl Views {
    /// The panel as an address, to find the card a click came from.
    pub fn key(&self) -> usize {
        Retained::as_ptr(&self.panel).cast::<()>() as usize
    }

    /// Empty the content view for a renderer.
    pub fn clear(&self) {
        for v in &self.content.subviews() {
            v.removeFromSuperview();
        }
    }

    /// Size every view to a card of `width` by `height`.
    pub fn resize(&self, width: f64, height: f64) {
        let bounds = rect(0.0, 0.0, width, height);
        self.root.setFrame(bounds);
        self.effect.setFrame(bounds);
        self.content.setFrame(bounds);
        self.close.setFrame(rect(width - CLOSE_INSET - CLOSE, CLOSE_INSET, CLOSE, CLOSE));
    }

    /// Move the panel to `frame` and bring it up, or hide it.
    pub fn place(&self, frame: Option<NSRect>) {
        place(&self.panel, frame);
    }
}

fn place(panel: &NSPanel, frame: Option<NSRect>) {
    match frame {
        Some(f) => {
            if panel.frame() != f {
                panel.setFrame_display(f, true);
                panel.invalidateShadow();
            }
            panel.orderFrontRegardless();
        }
        None => panel.orderOut(None),
    }
}

/// A card's `CardPanel` (`card`), or a plain panel (the pill).
fn panel(mtm: MainThreadMarker, card: bool) -> Retained<NSPanel> {
    let rect = rect(0.0, 0.0, 380.0, 100.0);
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel: Retained<NSPanel> = if card {
        // SAFETY: NSPanel's designated initializer, with argument types matching its signature.
        let p: Retained<CardPanel> = unsafe {
            msg_send![CardPanel::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
        };
        p.into_super()
    } else {
        // SAFETY: as above.
        unsafe {
            msg_send![NSPanel::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
        }
    };
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setLevel(LEVEL);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setHidesOnDeactivate(false);
    // SAFETY: the owner (`Views` or the pill) holds the panel until it closes it, and
    // `close` never frees it.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel
}

fn background(mtm: MainThreadMarker, radius: f64) -> Retained<NSVisualEffectView> {
    let effect = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), NSRect::ZERO);
    effect.setMaterial(NSVisualEffectMaterial::Popover);
    effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect.setState(NSVisualEffectState::Active);
    effect.setWantsLayer(true);
    if let Some(layer) = effect.layer() {
        layer.setCornerRadius(radius);
        layer.setMasksToBounds(true);
    }
    effect
}

/// A new, hidden card.
pub fn make(mtm: MainThreadMarker) -> Views {
    let panel = panel(mtm, true);
    // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
    let root: Retained<HudView> =
        unsafe { msg_send![HudView::alloc(mtm), initWithFrame: NSRect::ZERO] };
    // SAFETY: as above.
    let content: Retained<ContentView> =
        unsafe { msg_send![ContentView::alloc(mtm), initWithFrame: NSRect::ZERO] };
    let effect = background(mtm, 12.0);
    let close = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::ZERO);
    close.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    close.setContentTintColor(Some(&NSColor::tertiaryLabelColor()));
    let x = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &ns("xmark"),
        Some(&ns("Close")),
    );
    close.setImage(x.as_deref());
    for v in [&*effect as &NSView, &content, &close] {
        root.addSubview(v);
    }
    panel.setContentView(Some(&root));
    Views { panel, root, effect, content, close }
}

/// The `+N more` pill: a small panel that ignores the mouse.
pub struct Pill {
    panel: Retained<NSPanel>,
    text: Retained<NSTextField>,
}

impl Pill {
    pub fn new(mtm: MainThreadMarker) -> Pill {
        let panel = panel(mtm, false);
        panel.setIgnoresMouseEvents(true);
        let effect = background(mtm, super::stack::PILL_H / 2.0);
        let text = label(mtm, &NSFont::systemFontOfSize(11.0), &NSColor::secondaryLabelColor());
        text.setAlignment(NSTextAlignment::Center);
        effect.addSubview(&text);
        panel.setContentView(Some(&effect));
        Pill { panel, text }
    }

    /// Show `text` at `frame`, or hide.
    pub fn place(&self, shown: Option<(NSRect, &str)>) {
        if let Some((frame, text)) = shown {
            self.text.setStringValue(&ns(text));
            self.text.setFrame(rect(
                4.0,
                (frame.size.height - 15.0) / 2.0,
                frame.size.width - 8.0,
                15.0,
            ));
        }
        place(&self.panel, shown.map(|s| s.0));
    }
}
