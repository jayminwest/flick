//! The launcher panel: a borderless, non-activating `NSPanel` with a search field and result rows.
//! It shows what `render` gives it and reports typing and keys to the `Handlers`.

mod rows;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSBitmapImageFileType, NSColor, NSControl, NSControlTextEditingDelegate,
    NSEvent, NSFocusRingType, NSFont, NSFontWeightMedium, NSImage, NSPanel, NSResponder, NSScreen,
    NSTextAlignment, NSTextField, NSTextFieldDelegate, NSTextView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowCollectionBehavior, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    NSDate, NSDictionary, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop,
    NSSize,
};

pub use rows::{Frame, Icon, Row, render};
use rows::{RowViews, label, make_row, ns, separator, top_rect};

const W: f64 = 750.0;
const H: f64 = 474.0;
const SEARCH_H: f64 = 56.0;
const FOOTER_H: f64 = 40.0;
const ROW_H: f64 = 44.0;
const LIST_PAD: f64 = 6.0;
pub const VISIBLE_ROWS: usize = 8;
const STATUS_WINDOW_LEVEL: isize = 25;

/// A key command from the search field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    Enter,
    Tab,
    Escape,
    Backspace,
}

/// What the panel calls back into. `key` returns true when it handled the key.
#[derive(Clone, Copy)]
pub struct Handlers {
    pub query_changed: fn(),
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
        fn text_did_change(&self, _n: &NSNotification) {
            if let Some(h) = HANDLERS.get() {
                (h.query_changed)();
            }
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, _view: &NSTextView, sel: Sel) -> bool {
            // Unknown selectors fall through to the text view's default handling.
            key_for(sel).is_some_and(|key| HANDLERS.get().is_some_and(|h| (h.key)(key)))
        }

        #[unsafe(method(windowDidResignKey:))]
        fn did_resign_key(&self, _n: &NSNotification) {
            hide();
        }
    }
);

fn key_for(sel: Sel) -> Option<Key> {
    Some(if sel == sel!(moveUp:) {
        Key::Up
    } else if sel == sel!(moveDown:) {
        Key::Down
    } else if sel == sel!(insertNewline:) {
        Key::Enter
    } else if sel == sel!(insertTab:) {
        Key::Tab
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
    footer_left: Retained<NSTextField>,
    footer_action: Retained<NSTextField>,
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

    panel.setContentView(Some(&root));

    let ui = Ui {
        panel,
        field,
        rows,
        empty,
        footer_left,
        footer_action,
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
            let v = screen.visibleFrame();
            let x = v.origin.x + (v.size.width - W) / 2.0;
            let top = v.origin.y + v.size.height * 0.8;
            ui.panel.setFrameOrigin(NSPoint::new(x, (top - H).max(v.origin.y)));
        }
        ui.panel.makeKeyAndOrderFront(None);
        ui.panel.makeFirstResponder(Some(&ui.field));
    });
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
