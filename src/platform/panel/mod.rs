//! The launcher panel: a borderless, non-activating `NSPanel` with a search field and result rows.
//! It shows what `render` or `render_form` gives it and reports typing and keys to the
//! `Handlers`.

mod form;
mod keys;
mod rows;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSBitmapImageFileType, NSColor, NSControl, NSControlTextEditingDelegate,
    NSEvent, NSEventModifierFlags, NSEventType, NSFocusRingType, NSFont, NSFontWeightMedium,
    NSImage, NSPanel, NSResponder, NSScreen, NSTextAlignment, NSTextField, NSTextFieldDelegate,
    NSTextView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
    NSVisualEffectView, NSWindow, NSWindowCollectionBehavior, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    NSArray, NSDate, NSDictionary, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSRunLoop, NSSize,
};

use super::edit::{self, command_only};
pub use form::{FormField, FormFrame, field_value, focused_field, render_form};
use form::{FormViews, make_form};
pub use rows::{Frame, Icon, Row, render};
use rows::{RowViews, label, make_row, make_text, ns, separator, top_rect};

const W: f64 = 750.0;
const H: f64 = 474.0;
const SEARCH_H: f64 = 56.0;
pub(super) const FOOTER_H: f64 = 40.0;
const ROW_H: f64 = 44.0;
const LIST_PAD: f64 = 6.0;
pub const VISIBLE_ROWS: usize = 8;
const STATUS_WINDOW_LEVEL: isize = 25;

/// A key command from the search field, a form field or a read-only title.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    Enter,
    Tab,
    /// Shift-Tab.
    BackTab,
    Escape,
    Backspace,
    /// ⌘K.
    CmdK,
    /// ⌘↵.
    CmdEnter,
    /// Half a page up or down (⌃U, ⌃D with a read-only title).
    PageUp,
    PageDown,
    /// The first or last row (G, ⇧G with a read-only title).
    Top,
    Bottom,
}

/// What the panel calls back into. `key` returns true when it handled the key; an unhandled
/// key keeps its default behavior (⌘K and ⌘↵ go on to the menus).
#[derive(Clone, Copy)]
pub struct Handlers {
    pub query_changed: fn(),
    /// The text of form field `index` changed.
    pub field_changed: fn(usize),
    pub key: fn(Key) -> bool,
}

define_class!(
    // Borderless windows refuse key status by default; the search field needs it.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickPanel"]
    struct Panel;

    impl Panel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        // Vim-style navigation (`keys`), before the field editor turns ⌃K into a kill or J
        // into typing. A key the screen does not take goes on as usual.
        #[unsafe(method(sendEvent:))]
        fn send_event(&self, event: &NSEvent) {
            if !(event.r#type() == NSEventType::KeyDown && nav_key(event).is_some_and(send_key)) {
                // SAFETY: the superclass method, with the argument it was called with.
                unsafe { msg_send![super(self), sendEvent: event] }
            }
        }

        // ⌘K and ⌘↵ reach the window before the field editor sees them. Only handled keys
        // return true (as a tail expression: see mx-43d3f4). Flick has no main menu, so the
        // edit keys (⌘X, ⌘C, ⌘V, ⌘A, ⌘Z, ⇧⌘Z) go to the first responder here (flick-ab04).
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            let flags = event.modifierFlags();
            let chars = event.charactersIgnoringModifiers().map(|s| s.to_string());
            let chars = chars.as_deref().unwrap_or("");
            let key = key_equivalent(command_only(flags), chars, event.keyCode());
            key.is_some_and(send_key)
                || edit::send(event, self)
                // SAFETY: the superclass method, with the argument it was called with.
                || unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }

        // With a read-only title the panel itself is first responder: keys arrive here.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.interpretKeyEvents(&NSArray::from_slice(&[event]));
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, sel: Sel) {
            if !key_for(sel).is_some_and(send_key) {
                // SAFETY: the superclass method, with the argument it was called with.
                unsafe { msg_send![super(self), doCommandBySelector: sel] }
            }
        }

        // A read-only title ignores typing.
        #[unsafe(method(insertText:))]
        fn insert_text(&self, _text: &AnyObject) {}
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickDelegate"]
    struct Delegate;

    // SAFETY: the protocols' methods are optional; the ones below have their exact signatures.
    unsafe impl NSObjectProtocol for Delegate {}
    // SAFETY: as above.
    unsafe impl NSControlTextEditingDelegate for Delegate {}
    // SAFETY: as above.
    unsafe impl NSTextFieldDelegate for Delegate {}
    // SAFETY: as above.
    unsafe impl NSWindowDelegate for Delegate {}

    impl Delegate {
        #[unsafe(method(controlTextDidChange:))]
        fn text_did_change(&self, n: &NSNotification) {
            let field = n.object().and_then(|o| form::field_index(Retained::as_ptr(&o)));
            if let Some(h) = HANDLERS.get() {
                match field {
                    Some(index) => (h.field_changed)(index),
                    None => (h.query_changed)(),
                }
            }
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, control: &NSControl, view: &NSTextView, sel: Sel) -> bool {
            // Unknown selectors fall through to the text view's default handling.
            form::newline(std::ptr::from_ref(control).cast(), view, sel)
                || key_for(sel).is_some_and(send_key)
        }

        #[unsafe(method(windowDidResignKey:))]
        fn did_resign_key(&self, _n: &NSNotification) {
            hide();
        }
    }
);

/// The navigation key of key-down `event`, given whether the panel shows a read-only title.
fn nav_key(event: &NSEvent) -> Option<Key> {
    let flags = edit::held(event.modifierFlags());
    let held = if flags.is_empty() || flags == NSEventModifierFlags::Shift {
        keys::Held::Plain
    } else if flags == NSEventModifierFlags::Control {
        keys::Held::Control
    } else {
        keys::Held::Other
    };
    let chars = event.charactersIgnoringModifiers().map(|s| s.to_string());
    let read_only = with_ui(|ui| ui.form.mode.get() == form::Mode::Title).unwrap_or(false);
    keys::nav(chars.as_deref().unwrap_or(""), held, read_only)
}

fn send_key(key: Key) -> bool {
    HANDLERS.get().is_some_and(|h| (h.key)(key))
}

/// The key command for a key equivalent: `chars` ignores modifiers, `key_code` is the virtual
/// key (36 Return, 76 keypad Enter).
fn key_equivalent(command_only: bool, chars: &str, key_code: u16) -> Option<Key> {
    if !command_only {
        None
    } else if key_code == 36 || key_code == 76 {
        Some(Key::CmdEnter)
    } else if chars.eq_ignore_ascii_case("k") {
        Some(Key::CmdK)
    } else {
        None
    }
}

fn key_for(sel: Sel) -> Option<Key> {
    Some(if sel == sel!(moveUp:) {
        Key::Up
    } else if sel == sel!(moveDown:) {
        Key::Down
    } else if sel == sel!(insertNewline:) {
        Key::Enter
    } else if sel == sel!(insertTab:) {
        Key::Tab
    } else if sel == sel!(insertBacktab:) {
        Key::BackTab
    } else if sel == sel!(cancelOperation:) {
        Key::Escape
    } else if sel == sel!(deleteBackward:) {
        Key::Backspace
    } else {
        return None;
    })
}

struct Ui {
    panel: Retained<Panel>,
    field: Retained<NSTextField>,
    rows: Vec<RowViews>,
    empty: Retained<NSTextField>,
    text: Retained<NSTextField>,
    footer_left: Retained<NSTextField>,
    footer_action: Retained<NSTextField>,
    form: FormViews,
    icons: RefCell<HashMap<String, Retained<NSImage>>>,
    _delegate: Retained<Delegate>,
}

thread_local! {
    static UI: OnceCell<Ui> = const { OnceCell::new() };
    static HANDLERS: Cell<Option<Handlers>> = const { Cell::new(None) };
}

fn with_ui<R>(f: impl FnOnce(&Ui) -> R) -> Option<R> {
    UI.with(|ui| ui.get().map(f))
}

pub fn init(handlers: Handlers) {
    HANDLERS.set(Some(handlers));
    let mtm = super::mtm();
    // SAFETY: `init` is NSObject's designated initializer and returns a +1 object.
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    let rect = NSRect::new(NSPoint::ZERO, NSSize::new(W, H));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: NSPanel's designated initializer, with argument types matching its signature.
    let panel: Retained<Panel> = unsafe {
        msg_send![Panel::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    panel.setFloatingPanel(true);
    panel.setLevel(STATUS_WINDOW_LEVEL);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setHidesOnDeactivate(false);
    // SAFETY: `Ui` holds the panel for the life of the process, so close never frees it early.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    let root = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), rect);
    root.setMaterial(NSVisualEffectMaterial::Popover);
    root.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    root.setState(NSVisualEffectState::Active);
    root.setWantsLayer(true);
    if let Some(layer) = root.layer() {
        layer.setCornerRadius(12.0);
        layer.setMasksToBounds(true);
    }

    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        top_rect(H, 20.0, 14.0, W - 40.0, 28.0),
    );
    field.setBezeled(false);
    field.setBordered(false);
    field.setDrawsBackground(false);
    field.setFocusRingType(NSFocusRingType::None);
    field.setFont(Some(&NSFont::systemFontOfSize(20.0)));
    field.setTextColor(Some(&NSColor::labelColor()));
    field.setUsesSingleLineMode(true);
    // SAFETY: `Ui` keeps the delegate alive as long as the field (the field holds it weakly).
    unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    root.addSubview(&field);
    root.addSubview(&separator(mtm, top_rect(H, 0.0, SEARCH_H, W, 1.0)));

    let rows: Vec<RowViews> = (0..VISIBLE_ROWS).map(|i| make_row(mtm, i)).collect();
    for row in &rows {
        root.addSubview(&row.view);
    }

    let empty = label(mtm, 14.0, &NSColor::secondaryLabelColor());
    empty.setAlignment(NSTextAlignment::Center);
    empty.setFrame(top_rect(H, 0.0, f64::midpoint(H - FOOTER_H, SEARCH_H) - 10.0, W, 20.0));
    root.addSubview(&empty);
    let text = make_text(mtm);
    root.addSubview(&text);

    root.addSubview(&separator(mtm, top_rect(H, 0.0, H - FOOTER_H, W, 1.0)));
    let footer_left = label(mtm, 12.0, &NSColor::secondaryLabelColor());
    footer_left.setFrame(top_rect(H, 20.0, H - FOOTER_H + 12.0, 380.0, 16.0));
    let footer_action = label(mtm, 12.0, &NSColor::labelColor());
    // SAFETY: NSFontWeightMedium is an immutable framework constant, set at load time.
    let medium = unsafe { NSFontWeightMedium };
    footer_action.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, medium)));
    footer_action.setAlignment(NSTextAlignment::Right);
    footer_action.setFrame(top_rect(H, W - 20.0 - 300.0, H - FOOTER_H + 12.0, 300.0, 16.0));
    root.addSubview(&footer_left);
    root.addSubview(&footer_action);
    let form = make_form(mtm, &root, &delegate);

    panel.setContentView(Some(&root));

    let ui = Ui {
        panel,
        field,
        rows,
        empty,
        text,
        footer_left,
        footer_action,
        form,
        icons: RefCell::default(),
        _delegate: delegate,
    };
    UI.with(|cell| {
        let _ = cell.set(ui);
    });
}

/// Draw the panel into a PNG at `path` without showing it.
pub fn snapshot(path: &str) -> Result<(), String> {
    // App icons load asynchronously; let the run loop deliver them first.
    NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(1.0));
    with_ui(|ui| {
        let view = ui.panel.contentView().ok_or("no content view")?;
        let bounds = view.bounds();
        let rep =
            view.bitmapImageRepForCachingDisplayInRect(bounds).ok_or("can't create bitmap")?;
        view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
        // SAFETY: an empty properties dictionary is valid for PNG encoding.
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .ok_or("can't encode PNG")?;
        png.writeToFile_atomically(&ns(path), true)
            .then_some(())
            .ok_or(format!("can't write {path}"))
    })
    .unwrap_or(Err("UI not initialized".into()))
}

pub fn is_visible() -> bool {
    with_ui(|ui| ui.panel.isVisible()).unwrap_or(false)
}

/// Show on the screen under the mouse, upper-middle like Spotlight.
pub fn show() {
    place();
    with_ui(|ui| {
        ui.panel.makeKeyAndOrderFront(None);
        ui.panel.makeFirstResponder(form::responder(ui, ui.form.mode.get()));
    });
}

/// Move the panel to the upper middle of the screen under the mouse (else the main screen).
pub fn place() {
    let mtm = super::mtm();
    with_ui(|ui| {
        let mouse = NSEvent::mouseLocation();
        let screens = NSScreen::screens(mtm);
        let screen = screens.iter().find(|s| {
            let f = s.frame();
            mouse.x >= f.origin.x
                && mouse.x < f.origin.x + f.size.width
                && mouse.y >= f.origin.y
                && mouse.y < f.origin.y + f.size.height
        });
        if let Some(screen) = screen.or_else(|| NSScreen::mainScreen(mtm)) {
            ui.panel.setFrameOrigin(origin(screen.visibleFrame()));
        }
    });
}

/// The panel's origin in screen area `v`: centered, its top at 80% of the height, and its
/// bottom no lower than the area's.
fn origin(v: NSRect) -> NSPoint {
    let x = v.origin.x + (v.size.width - W) / 2.0;
    let top = v.origin.y + v.size.height * 0.8;
    NSPoint::new(x, (top - H).max(v.origin.y))
}

pub fn hide() {
    with_ui(|ui| ui.panel.orderOut(None));
}

pub fn query() -> String {
    with_ui(|ui| ui.field.stringValue().to_string()).unwrap_or_default()
}

pub fn set_query(text: &str, placeholder: &str) {
    with_ui(|ui| {
        ui.field.setStringValue(&ns(text));
        ui.field.setPlaceholderString(Some(&ns(placeholder)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_sits_upper_middle_and_stays_on_screen() {
        let area = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let p = origin(area(-1920.0, 100.0, 1920.0, 1000.0));
        assert!(near(p.x, -1920.0 + (1920.0 - W) / 2.0) && near(p.y, 900.0 - H), "{p:?}");
        // Too short for the 80% line: the bottom edge holds.
        assert!(near(origin(area(0.0, 25.0, 800.0, 500.0)).y, 25.0));
    }

    #[test]
    fn shift_tab_is_a_key_and_unknown_selectors_are_not() {
        assert_eq!(key_for(sel!(insertBacktab:)), Some(Key::BackTab));
        assert_eq!(key_for(sel!(insertTab:)), Some(Key::Tab));
        assert_eq!(key_for(sel!(selectAll:)), None);
    }

    #[test]
    fn cmd_k_and_cmd_enter_need_command_alone() {
        assert_eq!(key_equivalent(true, "k", 40), Some(Key::CmdK));
        assert_eq!(key_equivalent(true, "K", 40), Some(Key::CmdK));
        assert_eq!(key_equivalent(true, "\r", 36), Some(Key::CmdEnter));
        assert_eq!(key_equivalent(true, "\u{3}", 76), Some(Key::CmdEnter));
        // ⌘C, ⌘V and ⌘A are edit actions, not key commands.
        for c in ["c", "v", "a"] {
            assert_eq!(key_equivalent(true, c, 0), None);
        }
        assert_eq!(key_equivalent(false, "k", 40), None);
        assert_eq!(key_equivalent(false, "\r", 36), None);
    }
}
