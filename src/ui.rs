//! The launcher panel: a borderless, non-activating NSPanel with a search field and result rows.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBox, NSBoxType, NSColor, NSControl, NSControlTextEditingDelegate, NSEvent,
    NSFocusRingType, NSFont, NSFontWeightMedium, NSImage, NSImageScaling, NSImageView, NSLineBreakMode,
    NSPanel, NSResponder, NSScreen, NSTextAlignment, NSTextField, NSTextFieldDelegate, NSTextView, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowCollectionBehavior, NSWindowDelegate, NSWorkspace, NSWindowStyleMask,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer};

use crate::search::{Icon, Item};

const W: f64 = 750.0;
const H: f64 = 474.0;
const SEARCH_H: f64 = 56.0;
const FOOTER_H: f64 = 40.0;
const ROW_H: f64 = 44.0;
const LIST_PAD: f64 = 6.0;
pub const VISIBLE_ROWS: usize = 8;
const STATUS_WINDOW_LEVEL: isize = 25;

define_class!(
    // Borderless windows refuse key status by default; the search field needs it.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickPanel"]
    pub struct Panel;

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

    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSControlTextEditingDelegate for Delegate {}
    unsafe impl NSTextFieldDelegate for Delegate {}
    unsafe impl NSWindowDelegate for Delegate {}

    impl Delegate {
        #[unsafe(method(controlTextDidChange:))]
        fn text_did_change(&self, _n: &NSNotification) {
            crate::app::query_changed();
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, _view: &NSTextView, sel: Sel) -> bool {
            crate::app::command(sel)
        }

        #[unsafe(method(windowDidResignKey:))]
        fn did_resign_key(&self, _n: &NSNotification) {
            hide();
        }
    }
);

struct Row {
    view: Retained<NSView>,
    bg: Retained<NSBox>,
    icon: Retained<NSImageView>,
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
    accessory: Retained<NSTextField>,
}

pub struct Ui {
    panel: Retained<Panel>,
    field: Retained<NSTextField>,
    rows: Vec<Row>,
    empty: Retained<NSTextField>,
    footer_left: Retained<NSTextField>,
    footer_action: Retained<NSTextField>,
    icons: RefCell<HashMap<String, Retained<NSImage>>>,
    _delegate: Retained<Delegate>,
}

thread_local! {
    static UI: OnceCell<Ui> = const { OnceCell::new() };
}

fn with_ui<R>(f: impl FnOnce(&Ui) -> R) -> Option<R> {
    UI.with(|ui| ui.get().map(f))
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// A frame measured from the top-left of a parent of height `parent_h`.
fn top_rect(parent_h: f64, x: f64, top: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, parent_h - top - h), NSSize::new(w, h))
}

fn label(mtm: MainThreadMarker, size: f64, color: &NSColor) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(""), mtm);
    l.setFont(Some(&NSFont::systemFontOfSize(size)));
    l.setTextColor(Some(color));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

fn separator(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSBox> {
    let b = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    b.setBoxType(NSBoxType::Separator);
    b
}

fn make_row(mtm: MainThreadMarker, index: usize) -> Row {
    let top = SEARCH_H + LIST_PAD + index as f64 * ROW_H;
    let view = NSView::initWithFrame(NSView::alloc(mtm), top_rect(H, 8.0, top, W - 16.0, ROW_H));

    let bg = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::new(NSPoint::ZERO, NSSize::new(W - 16.0, ROW_H)));
    bg.setBoxType(NSBoxType::Custom);
    bg.setBorderWidth(0.0);
    bg.setCornerRadius(8.0);
    bg.setFillColor(&NSColor::labelColor().colorWithAlphaComponent(0.1));
    bg.setTransparent(true);

    let icon = NSImageView::initWithFrame(NSImageView::alloc(mtm), top_rect(ROW_H, 12.0, 11.0, 22.0, 22.0));
    icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);

    let title = label(mtm, 14.0, &NSColor::labelColor());
    let subtitle = label(mtm, 13.0, &NSColor::secondaryLabelColor());
    let accessory = label(mtm, 13.0, &NSColor::tertiaryLabelColor());
    accessory.setAlignment(NSTextAlignment::Right);
    accessory.setFrame(top_rect(ROW_H, W - 16.0 - 12.0 - 160.0, 13.0, 160.0, 18.0));

    for v in [&*bg as &NSView, &icon, &title, &subtitle, &accessory] {
        view.addSubview(v);
    }
    view.setHidden(true);
    Row { view, bg, icon, title, subtitle, accessory }
}

pub fn init(mtm: MainThreadMarker) {
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    let rect = NSRect::new(NSPoint::ZERO, NSSize::new(W, H));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel: Retained<Panel> = unsafe {
        msg_send![Panel::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    panel.setFloatingPanel(true);
    panel.setLevel(STATUS_WINDOW_LEVEL);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setHidesOnDeactivate(false);
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

    let field = NSTextField::initWithFrame(NSTextField::alloc(mtm), top_rect(H, 20.0, 14.0, W - 40.0, 28.0));
    field.setBezeled(false);
    field.setBordered(false);
    field.setDrawsBackground(false);
    field.setFocusRingType(NSFocusRingType::None);
    field.setFont(Some(&NSFont::systemFontOfSize(20.0)));
    field.setUsesSingleLineMode(true);
    unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    root.addSubview(&field);
    root.addSubview(&separator(mtm, top_rect(H, 0.0, SEARCH_H, W, 1.0)));

    let rows: Vec<Row> = (0..VISIBLE_ROWS).map(|i| make_row(mtm, i)).collect();
    for row in &rows {
        root.addSubview(&row.view);
    }

    let empty = label(mtm, 14.0, &NSColor::secondaryLabelColor());
    empty.setAlignment(NSTextAlignment::Center);
    empty.setFrame(top_rect(H, 0.0, (H - FOOTER_H + SEARCH_H) / 2.0 - 10.0, W, 20.0));
    root.addSubview(&empty);

    root.addSubview(&separator(mtm, top_rect(H, 0.0, H - FOOTER_H, W, 1.0)));
    let footer_left = label(mtm, 12.0, &NSColor::secondaryLabelColor());
    footer_left.setFrame(top_rect(H, 20.0, H - FOOTER_H + 12.0, 380.0, 16.0));
    let footer_action = label(mtm, 12.0, &NSColor::labelColor());
    footer_action.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, unsafe { NSFontWeightMedium })));
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

pub fn is_visible() -> bool {
    with_ui(|ui| ui.panel.isVisible()).unwrap_or(false)
}

/// Show on the screen under the mouse, upper-middle like Spotlight.
pub fn show(mtm: MainThreadMarker) {
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

fn icon_image(ui: &Ui, icon: &Icon) -> Option<Retained<NSImage>> {
    let key = match icon {
        Icon::File(p) => p.display().to_string(),
        Icon::Symbol(s) => format!("sf:{s}"),
    };
    if let Some(img) = ui.icons.borrow().get(&key) {
        return Some(img.clone());
    }
    let img = match icon {
        Icon::File(p) => NSWorkspace::sharedWorkspace().iconForFile(&ns(&p.display().to_string())),
        Icon::Symbol(s) => NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(s), None)
            .or_else(|| NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns("app"), None))?,
    };
    ui.icons.borrow_mut().insert(key, img.clone());
    Some(img)
}

pub struct View<'a> {
    pub items: &'a [Item],
    pub selected: usize,
    pub scroll: usize,
    pub footer: &'a str,
    pub empty: &'a str,
}

pub fn render(view: &View) {
    with_ui(|ui| {
        let visible = view.items.iter().enumerate().skip(view.scroll).take(VISIBLE_ROWS);
        let mut shown = 0;
        for (row, (index, item)) in ui.rows.iter().zip(visible) {
            shown += 1;
            row.view.setHidden(false);
            row.bg.setTransparent(index != view.selected);

            row.icon.setImage(icon_image(ui, &item.icon).as_deref());
            let tint = matches!(item.icon, Icon::Symbol(_)).then(NSColor::secondaryLabelColor);
            row.icon.setContentTintColor(tint.as_deref());

            let title_x = 46.0;
            let max_text = W - 16.0 - title_x - 12.0 - 170.0;
            row.title.setStringValue(&ns(&item.title));
            let title_w = row.title.cell().map_or(0.0, |c| c.cellSize().width).min(max_text).ceil();
            row.title.setFrame(top_rect(ROW_H, title_x, 13.0, title_w, 18.0));

            row.subtitle.setStringValue(&ns(&item.subtitle));
            let sub_w = (max_text - title_w - 8.0).max(0.0);
            row.subtitle.setFrame(top_rect(ROW_H, title_x + title_w + 8.0, 13.5, sub_w, 17.0));

            row.accessory.setStringValue(&ns(&item.accessory));
        }
        for row in &ui.rows[shown..] {
            row.view.setHidden(true);
        }

        ui.empty.setStringValue(&ns(if view.items.is_empty() { view.empty } else { "" }));
        ui.footer_left.setStringValue(&ns(view.footer));
        let action = view.items.get(view.selected).map(|i| format!("{}  ↵", i.action.verb()));
        ui.footer_action.setStringValue(&ns(action.as_deref().unwrap_or("")));
    });
}

/// Run `f` on the main run loop after `secs`.
pub fn after(secs: f64, f: impl Fn() + 'static) {
    let block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| f());
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(secs, false, &block) };
}

/// Run `f` on the main run loop every `secs`.
pub fn every(secs: f64, f: impl Fn() + 'static) {
    let block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| f());
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(secs, true, &block) };
}
