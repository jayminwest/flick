//! The `AppKit` side of a surface: the key-capable non-activating panel class, its flipped
//! blurred content view, the one shared delegate (window, text view and chip target), and
//! building a surface's views. Callbacks find their surface by window or text view and go
//! through `super::handle` and friends.

use std::cell::OnceCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSBezelStyle, NSBox, NSBoxType, NSButton,
    NSCellImagePosition, NSColor, NSControlSize, NSEvent, NSEventModifierFlags, NSFont,
    NSFontWeightSemibold, NSImage, NSLineBreakMode, NSPanel, NSResponder, NSScreen, NSTextDelegate,
    NSTextField, NSTextView, NSTextViewDelegate, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowCollectionBehavior, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSRect, NSSize, NSString};

use super::geometry::{Rect, Screen, Size};
use super::input::{Field, filled, ns_rect};
use super::keys::{self, Mods};
use super::transcript::Transcript;
use super::{Chip, Input, Key, Keystroke, Spec, by_text, by_window, close, handle, relayout};
use crate::platform::{edit, timer};

/// Floating, like the launcher panel.
const LEVEL: isize = 25;

define_class!(
    // Borderless windows refuse key status by default; the input needs it.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickSurfacePanel"]
    pub(super) struct SurfacePanel;

    impl SurfacePanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        // Edit keys first (Flick has no Edit menu, mx-43d850), then the module's ⌘ keys.
        // Tail expression only (mx-43d3f4).
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            self.isKeyWindow()
                && (edit::send(event, self) || key_equivalent(self.as_ref(), event))
                // SAFETY: the superclass method, with the argument it was called with.
                || unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }

        // Escape when the panel itself has focus (after a click on the background).
        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) {
            if let Some(s) = by_window(self.as_ref()) {
                let k = Keystroke { key: Key::Escape, cmd: false, shift: false, opt: false };
                handle(&s, k);
            }
        }
    }
);

define_class!(
    // The content view: a blurred background, flipped so layouts read top-down.
    #[unsafe(super(NSVisualEffectView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickSurfaceView"]
    pub(super) struct RootView;

    impl RootView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

define_class!(
    // The one window and text delegate and chip target of every surface.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickSurfaceDelegate"]
    pub(super) struct Delegate;

    // SAFETY: the protocols' methods are optional; the ones below have their exact signatures.
    unsafe impl NSObjectProtocol for Delegate {}
    // SAFETY: as above.
    unsafe impl NSWindowDelegate for Delegate {}
    // SAFETY: as above.
    unsafe impl NSTextDelegate for Delegate {}
    // SAFETY: as above.
    unsafe impl NSTextViewDelegate for Delegate {}

    impl Delegate {
        #[unsafe(method(windowDidResize:))]
        fn did_resize(&self, n: &NSNotification) {
            if let Some(s) = n.object().and_then(|o| by_window(&o)) {
                relayout(&s);
            }
        }

        #[unsafe(method(windowDidResignKey:))]
        fn did_resign_key(&self, n: &NSNotification) {
            if let Some(s) = n.object().and_then(|o| by_window(&o))
                && s.hide_on_blur
                && s.v.panel.isVisible()
            {
                close(&s);
            }
        }

        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, n: &NSNotification) {
            let view = n.object().and_then(|o| o.downcast::<NSTextView>().ok());
            if let Some(s) = view.as_deref().and_then(by_text) {
                s.v.field.sync();
            }
        }

        // Return, Escape, Up and Down in the input. Unknown commands keep their default.
        #[unsafe(method(textView:doCommandBySelector:))]
        fn do_command(&self, view: &NSTextView, command: Sel) -> bool {
            let name = command.name().to_str().unwrap_or("");
            let keystroke = keys::command(name, current_mods(view.mtm()));
            match (by_text(view), keystroke) {
                (Some(s), Some(k)) => handle(&s, k),
                _ => false,
            }
        }

        // Deferred: the module redraws the chips, removing the sender mid-action.
        #[unsafe(method(chipPressed:))]
        fn chip_pressed(&self, sender: &NSButton) {
            let index = usize::try_from(sender.tag()).unwrap_or(usize::MAX);
            if let Some(s) = sender.window().and_then(|w| by_window(&w)) {
                let (id, h) = (s.id, s.handlers);
                timer::after(0.0, move || (h.chip_removed)(id, index));
            }
        }
    }
);

thread_local! {
    static DELEGATE: OnceCell<Retained<Delegate>> = const { OnceCell::new() };
}

pub(super) fn delegate(mtm: MainThreadMarker) -> Retained<Delegate> {
    DELEGATE.with(|d| {
        d.get_or_init(|| {
            // SAFETY: `init` is NSObject's designated initializer and returns a +1 object.
            unsafe { msg_send![Delegate::alloc(mtm), init] }
        })
        .clone()
    })
}

pub(super) fn mods(flags: NSEventModifierFlags) -> Mods {
    Mods {
        cmd: flags.contains(NSEventModifierFlags::Command),
        shift: flags.contains(NSEventModifierFlags::Shift),
        opt: flags.contains(NSEventModifierFlags::Option),
    }
}

/// The modifiers of the event being handled.
fn current_mods(mtm: MainThreadMarker) -> Mods {
    let event = NSApplication::sharedApplication(mtm).currentEvent();
    event.map(|e| mods(e.modifierFlags())).unwrap_or_default()
}

fn key_equivalent(window: &AnyObject, event: &NSEvent) -> bool {
    let chars = event.charactersIgnoringModifiers().map(|s| s.to_string());
    let k = keys::equivalent(
        chars.as_deref().unwrap_or(""),
        event.keyCode(),
        mods(event.modifierFlags()),
    );
    match (by_window(window), k) {
        (Some(s), Some(k)) => handle(&s, k),
        _ => false,
    }
}

pub(super) fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn label(mtm: MainThreadMarker, font: &NSFont, color: &NSColor) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(""), mtm);
    l.setFont(Some(font));
    l.setTextColor(Some(color));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

/// A surface's views.
pub(super) struct Views {
    pub(super) panel: Retained<SurfacePanel>,
    pub(super) root: Retained<RootView>,
    pub(super) title: Retained<NSTextField>,
    pub(super) subtitle: Retained<NSTextField>,
    pub(super) dot: Retained<NSBox>,
    pub(super) rule: Retained<NSBox>,
    /// The rows area.
    pub(super) rows: Transcript,
    pub(super) notice: Retained<NSTextField>,
    pub(super) field: Field,
}

/// The hidden panel and views for `spec`, at its autosaved frame if there is one.
pub(super) fn build(mtm: MainThreadMarker, spec: &Spec, min: Size) -> Views {
    let delegate = delegate(mtm);
    let size = Size { w: spec.size.w.max(min.w), h: spec.size.h.max(min.h) };
    let frame = ns_rect(Rect::new(0.0, 0.0, size.w, size.h));
    let style = NSWindowStyleMask::Borderless
        | NSWindowStyleMask::NonactivatingPanel
        | NSWindowStyleMask::Resizable;
    // SAFETY: NSPanel's designated initializer, with argument types matching its signature.
    let panel: Retained<SurfacePanel> = unsafe {
        msg_send![SurfacePanel::alloc(mtm), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    panel.setFloatingPanel(true);
    panel.setLevel(LEVEL);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setHidesOnDeactivate(false);
    panel.setMovableByWindowBackground(true);
    panel.setContentMinSize(NSSize::new(min.w, min.h));
    // SAFETY: the surface map holds the panel for the life of the process; it is never freed.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
    let root: Retained<RootView> = unsafe { msg_send![RootView::alloc(mtm), initWithFrame: frame] };
    root.setMaterial(NSVisualEffectMaterial::Popover);
    root.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    root.setState(NSVisualEffectState::Active);
    root.setWantsLayer(true);
    if let Some(layer) = root.layer() {
        layer.setCornerRadius(12.0);
        layer.setMasksToBounds(true);
    }

    // SAFETY: NSFontWeightSemibold is an immutable framework constant, set at load time.
    let semibold = unsafe { NSFontWeightSemibold };
    let title =
        label(mtm, &NSFont::systemFontOfSize_weight(14.0, semibold), &NSColor::labelColor());
    title.setStringValue(&ns(spec.title));
    let small = NSFont::systemFontOfSize(12.0);
    let subtitle = label(mtm, &small, &NSColor::secondaryLabelColor());
    let notice = label(mtm, &small, &NSColor::secondaryLabelColor());
    notice.setHidden(true);
    let dot = filled(mtm, &NSColor::tertiaryLabelColor(), 4.0);
    // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
    let rule: Retained<NSBox> =
        unsafe { msg_send![NSBox::alloc(mtm), initWithFrame: NSRect::ZERO] };
    rule.setBoxType(NSBoxType::Separator);
    let rows = Transcript::new(mtm);
    for v in [&*title as &NSView, &subtitle, &dot, &rule, rows.view(), &notice] {
        root.addSubview(v);
    }
    let field = Field::new(mtm, &root, &delegate, spec.placeholder, spec.input == Input::Multi);
    panel.setContentView(Some(&root));

    if !spec.autosave.is_empty() {
        let name = ns(spec.autosave);
        if !panel.setFrameUsingName(&name) {
            panel.center();
        }
        panel.setFrameAutosaveName(&name);
    }
    Views { panel, root, title, subtitle, dot, rule, rows, notice, field }
}

/// Every display, in `AppKit` screen coordinates.
pub(super) fn screens(mtm: MainThreadMarker) -> Vec<Screen> {
    let rect = |r: NSRect| Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height);
    NSScreen::screens(mtm)
        .iter()
        .map(|s| Screen { frame: rect(s.frame()), visible: rect(s.visibleFrame()) })
        .collect()
}

/// A chip: its symbol, label and a remove mark, reporting `chipPressed:` with tag `index`.
pub(super) fn chip_button(mtm: MainThreadMarker, chip: &Chip, index: usize) -> Retained<NSButton> {
    let target = delegate(mtm);
    let title = ns(&format!("{}  \u{d7}", chip.label));
    // SAFETY: `chipPressed:` is a method of the delegate, which lives for the process.
    let b = unsafe {
        NSButton::buttonWithTitle_target_action(
            &title,
            Some(&target),
            Some(sel!(chipPressed:)),
            mtm,
        )
    };
    b.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    b.setControlSize(NSControlSize::Small);
    b.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    b.setTag(index as isize);
    b.setToolTip(Some(&ns("Remove")));
    let image = (!chip.symbol.is_empty())
        .then(|| {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(chip.symbol), None)
        })
        .flatten();
    if let Some(image) = image {
        b.setImage(Some(&image));
        b.setImagePosition(NSCellImagePosition::ImageLeading);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_flags_become_mods() {
        use NSEventModifierFlags as F;
        let m = mods(F::Command | F::Shift | F::CapsLock);
        assert_eq!(m, Mods { cmd: true, shift: true, opt: false });
        assert_eq!(mods(F::Option), Mods { cmd: false, shift: false, opt: true });
        assert_eq!(mods(F::empty()), Mods::default());
    }
}
