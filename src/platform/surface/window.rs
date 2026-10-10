//! The `AppKit` side of a surface: the key-capable non-activating panel class, its flipped
//! blurred content view, the one shared delegate (window, text view and chip target), and
//! building a surface's views. Callbacks find their surface by window or text view and go
//! through `super::handle` and friends.
//!
//! A private surface (`Spec::private`): `sharingType` none (left out of screenshots, screen
//! recording and window sharing), not restorable, out of the Windows menu, its window title
//! fixed to the spec's title, a banner strip and accent, an input with every text service
//! off (`privacy::ALL`) and no context menu, bubbles that cannot be selected, and ⌘C/⌘X
//! that copy only through `pasteboard::set_text_concealed` (`privacy::edit`).

use std::cell::OnceCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSBezelStyle, NSBox, NSBoxType, NSButton,
    NSCellImagePosition, NSColor, NSControlSize, NSEvent, NSEventModifierFlags, NSFont,
    NSFontWeightSemibold, NSImage, NSLineBreakMode, NSMenu, NSPanel, NSResponder, NSScreen,
    NSTextAlignment, NSTextDelegate, NSTextField, NSTextView, NSTextViewDelegate, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindow, NSWindowCollectionBehavior, NSWindowDelegate, NSWindowSharingType, NSWindowStyleMask,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSRect, NSSize, NSString};

use super::geometry::{Rect, Screen, Size};
use super::input::{Field, accent, filled, ns_rect};
use super::keys::{self, Mods};
use super::privacy::{self, Edit};
use super::transcript::Transcript;
use super::{Chip, Input, Key, Keystroke, Spec, by_text, by_window, close, handle, relayout};
use crate::platform::{edit, pasteboard, timer};

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

        // Edit keys first (Flick has no Edit menu, mx-43d850; a private surface takes ⌘C/⌘X
        // itself), then the module's ⌘ keys. Tail expression only (mx-43d3f4).
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            self.isKeyWindow()
                && (private_edit(self, event)
                    || edit::send(event, self)
                    || key_equivalent(self.as_ref(), event))
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

        // A private surface's input has no context menu: its Copy, Services, Look Up and
        // Writing Tools items would carry the text out. Tail expression only (mx-43d3f4).
        #[unsafe(method_id(textView:menu:forEvent:atIndex:))]
        fn text_menu(
            &self,
            view: &NSTextView,
            menu: &NSMenu,
            _event: &NSEvent,
            _index: usize,
        ) -> Option<Retained<NSMenu>> {
            let private = view.window().and_then(|w| by_window(&w)).is_some_and(|s| s.private);
            if private { None } else { Some(menu.retain()) }
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

/// ⌘C/⌘X in a private surface: copy the selection concealed (and delete it for ⌘X), or,
/// with nothing selected, offer the key to the module. False for every other key and for
/// normal surfaces (`edit::send` handles those).
fn private_edit(panel: &SurfacePanel, event: &NSEvent) -> bool {
    let Some(s) = by_window(panel.as_ref()).filter(|s| s.private) else { return false };
    let flags = event.modifierFlags();
    let chars = event.charactersIgnoringModifiers().map(|c| c.to_string()).unwrap_or_default();
    let action = edit::edit_action(edit::command_only(flags), edit::command_shift(flags), &chars);
    let text = panel.firstResponder().and_then(|r| r.downcast::<NSTextView>().ok());
    let range = text.as_ref().map(|t| t.selectedRange());
    let name = action.map_or("", |a| a.name().to_str().unwrap_or(""));
    match (privacy::edit(name, range.map_or(0, |r| r.length)), text, range) {
        (Edit::Pass, ..) => false,
        (Edit::Conceal { cut }, Some(text), Some(range)) => {
            let selected = text.string().substringWithRange(range).to_string();
            pasteboard::set_text_concealed(&selected);
            // Best effort: zero this copy before it is freed.
            let mut bytes = selected.into_bytes();
            bytes.fill(0);
            std::hint::black_box(&bytes);
            if cut && text.isEditable() {
                // SAFETY: `delete:` with no sender, on the main thread.
                unsafe { text.delete(None) };
            }
            true
        }
        _ => {
            if let Some(k) = keys::equivalent(&chars, event.keyCode(), mods(flags)) {
                handle(&s, k);
            }
            true
        }
    }
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
    /// A private surface's banner: its fill and its label.
    pub(super) banner: Option<(Retained<NSBox>, Retained<NSTextField>)>,
}

/// A private surface's banner strip: `text` in white on the accent.
fn banner(mtm: MainThreadMarker, text: &str) -> (Retained<NSBox>, Retained<NSTextField>) {
    let back = filled(mtm, &accent(), 0.0);
    let small = NSFont::systemFontOfSize_weight(
        11.0,
        // SAFETY: NSFontWeightSemibold is an immutable framework constant, set at load time.
        unsafe { NSFontWeightSemibold },
    );
    let l = label(mtm, &small, &NSColor::whiteColor());
    l.setAlignment(NSTextAlignment::Center);
    l.setStringValue(&ns(text));
    (back, l)
}

/// Keep a private panel out of captures, shares, restoration and the Windows menu, under a
/// fixed title.
fn seal(panel: &SurfacePanel, title: &str) {
    panel.setSharingType(NSWindowSharingType::None);
    panel.setRestorable(false);
    panel.setExcludedFromWindowsMenu(true);
    panel.setTitle(&ns(title));
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
    let private = spec.private.is_some();
    if private {
        seal(&panel, spec.title);
    }

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
    let rows = Transcript::new(mtm, !private);
    for v in [&*title as &NSView, &subtitle, &dot, &rule, rows.view(), &notice] {
        root.addSubview(v);
    }
    let banner = spec.private.map(|text| banner(mtm, text));
    if let Some((back, l)) = &banner {
        root.addSubview(back);
        root.addSubview(l);
    }
    let multi = spec.input == Input::Multi;
    let field = Field::new(mtm, &root, &delegate, spec.placeholder, multi, private);
    panel.setContentView(Some(&root));

    if !spec.autosave.is_empty() {
        let name = ns(spec.autosave);
        if !panel.setFrameUsingName(&name) {
            panel.center();
        }
        panel.setFrameAutosaveName(&name);
    }
    Views { panel, root, title, subtitle, dot, rule, rows, notice, field, banner }
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
