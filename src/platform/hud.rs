//! The message HUD: a small borderless, non-activating `NSPanel` in a screen corner that shows
//! one card (header, time, an optional context line, a wrapping body and a hint line). It
//! never becomes key and never activates Flick, so it does not take focus from the app in
//! front. A click opens the card's link (if any) and dismisses it; Escape dismisses it (a
//! global key monitor, which needs Accessibility, installed only while it shows); it hides
//! by itself after the timeout unless the pointer is over it.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSFont, NSFontWeightSemibold, NSImage,
    NSImageScaling, NSImageView, NSLineBreakMode, NSPanel, NSResponder, NSScreen, NSSound,
    NSTextAlignment, NSTextField, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};

use super::{timer, workspace};

const PAD: f64 = 14.0;
const GAP: f64 = 12.0;
const HEADER_H: f64 = 18.0;
const LINE_H: f64 = 15.0;
const MAX_BODY_H: f64 = 360.0;
const LEVEL: isize = 25;
const ESCAPE: u16 = 53;
/// Seconds the pointer over the card holds off the timeout, rechecked each time.
const HOVER_RECHECK: f64 = 2.0;

/// Where the card sits in the visible area of the screen under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
    Top,
    Bottom,
}

/// What the card shows.
pub struct Card<'a> {
    /// Bold, top left: the sender or the message title.
    pub header: &'a str,
    /// Top right, e.g. "14:03".
    pub time: &'a str,
    /// One secondary line under the header (e.g. the question this replies to); empty: none.
    pub context: &'a str,
    pub body: &'a str,
    /// Opened by a click.
    pub link: Option<&'a str>,
    /// A placeholder waiting for its reply: dimmed body, an hourglass, no sound.
    pub pending: bool,
}

/// How and where the card shows.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub corner: Corner,
    pub width: f64,
    /// Seconds before it hides; 0 keeps it until dismissed.
    pub timeout_secs: f64,
    pub sound: bool,
}

define_class!(
    // The content view: flipped, so the layout reads top-down, and it takes every click.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickHudView"]
    struct HudView;

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

        // Labels never swallow the click: the whole card is one target.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            (std::ptr::from_ref(self)).cast::<NSView>().cast_mut()
        }

        // Defer the work: it hides the panel this event is still being delivered to.
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            timer::after(0.0, clicked);
        }
    }
);

struct Views {
    panel: Retained<NSPanel>,
    root: Retained<HudView>,
    effect: Retained<NSVisualEffectView>,
    icon: Retained<NSImageView>,
    header: Retained<NSTextField>,
    time: Retained<NSTextField>,
    context: Retained<NSTextField>,
    body: Retained<NSTextField>,
    hint: Retained<NSTextField>,
}

#[derive(Default)]
struct State {
    views: Option<Views>,
    link: Option<String>,
    monitors: Vec<Retained<AnyObject>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
    /// Bumped by every show and hide, so a stale timeout does nothing.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn label(mtm: MainThreadMarker, font: &NSFont, color: &NSColor) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(""), mtm);
    l.setFont(Some(font));
    l.setTextColor(Some(color));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

fn make(mtm: MainThreadMarker) -> Views {
    let rect = NSRect::new(NSPoint::ZERO, NSSize::new(380.0, 100.0));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: NSPanel's designated initializer, with argument types matching its signature.
    let panel: Retained<NSPanel> = unsafe {
        msg_send![NSPanel::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
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
    // SAFETY: `State` holds the panel for the life of the process, so close never frees it.
    unsafe { panel.setReleasedWhenClosed(false) };

    // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
    let root: Retained<HudView> = unsafe { msg_send![HudView::alloc(mtm), initWithFrame: rect] };
    let effect = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), rect);
    effect.setMaterial(NSVisualEffectMaterial::Popover);
    effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect.setState(NSVisualEffectState::Active);
    effect.setWantsLayer(true);
    if let Some(layer) = effect.layer() {
        layer.setCornerRadius(12.0);
        layer.setMasksToBounds(true);
    }
    let icon = NSImageView::initWithFrame(NSImageView::alloc(mtm), rect);
    icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
    // SAFETY: NSFontWeightSemibold is an immutable framework constant, set at load time.
    let semibold = unsafe { NSFontWeightSemibold };
    let header =
        label(mtm, &NSFont::systemFontOfSize_weight(13.0, semibold), &NSColor::labelColor());
    let time = label(mtm, &NSFont::systemFontOfSize(11.0), &NSColor::tertiaryLabelColor());
    time.setAlignment(NSTextAlignment::Right);
    let context = label(mtm, &NSFont::systemFontOfSize(11.0), &NSColor::secondaryLabelColor());
    let body = NSTextField::wrappingLabelWithString(&ns(""), mtm);
    body.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    body.setSelectable(false);
    if let Some(cell) = body.cell() {
        cell.setTruncatesLastVisibleLine(true);
    }
    let hint = label(mtm, &NSFont::systemFontOfSize(11.0), &NSColor::tertiaryLabelColor());
    for v in [&*effect as &NSView, &icon, &header, &time, &context, &body, &hint] {
        root.addSubview(v);
    }
    panel.setContentView(Some(&root));
    Views { panel, root, effect, icon, header, time, context, body, hint }
}

/// The card's origin in screen area `v` for a card of `size`, `GAP` in from the edges.
fn origin(v: NSRect, size: NSSize, corner: Corner) -> NSPoint {
    let left = v.origin.x + GAP;
    let right = v.origin.x + v.size.width - size.width - GAP;
    let center = v.origin.x + (v.size.width - size.width) / 2.0;
    let top = v.origin.y + v.size.height - size.height - GAP;
    let bottom = v.origin.y + GAP;
    let (x, y) = match corner {
        Corner::TopRight => (right, top),
        Corner::TopLeft => (left, top),
        Corner::BottomRight => (right, bottom),
        Corner::BottomLeft => (left, bottom),
        Corner::Top => (center, top),
        Corner::Bottom => (center, bottom),
    };
    NSPoint::new(x, y)
}

/// The hint line under the body.
fn hint(card: &Card) -> String {
    match (card.pending, card.link) {
        (true, _) => "Waiting for a reply…  ·  esc to dismiss".into(),
        (false, Some(link)) => format!("Click to open {}  ·  esc to dismiss", host(link)),
        (false, None) => "Click or esc to dismiss".into(),
    }
}

/// `https://example.com/a/b` → `example.com`; anything else as is.
fn host(link: &str) -> &str {
    let rest = link.split_once("://").map_or(link, |(_, r)| r);
    rest.split('/').next().filter(|h| !h.is_empty()).unwrap_or(link)
}

/// Lay out `card` in `v` at `width` and return the card's height.
fn layout(v: &Views, card: &Card, width: f64) -> f64 {
    let inner = width - 2.0 * PAD;
    let symbol = if card.pending { "hourglass" } else { "bubble.left.fill" };
    v.icon.setImage(
        NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(symbol), None).as_deref(),
    );
    v.icon.setFrame(rect(PAD, PAD + 1.0, 16.0, 16.0));
    v.header.setStringValue(&ns(card.header));
    v.header.setFrame(rect(PAD + 22.0, PAD, inner - 22.0 - 70.0, HEADER_H));
    v.time.setStringValue(&ns(card.time));
    v.time.setFrame(rect(width - PAD - 66.0, PAD + 2.0, 66.0, LINE_H));
    let mut y = PAD + HEADER_H + 4.0;
    v.context.setHidden(card.context.is_empty());
    if !card.context.is_empty() {
        v.context.setStringValue(&ns(card.context));
        v.context.setFrame(rect(PAD, y, inner, LINE_H));
        y += LINE_H + 4.0;
    }
    v.body.setStringValue(&ns(card.body));
    let color = if card.pending { NSColor::secondaryLabelColor() } else { NSColor::labelColor() };
    v.body.setTextColor(Some(&color));
    v.body.setPreferredMaxLayoutWidth(inner);
    let big = NSRect::new(NSPoint::ZERO, NSSize::new(inner, 10_000.0));
    let measured = v.body.cell().map_or(LINE_H, |c| c.cellSizeForBounds(big).height);
    let body_h = measured.ceil().min(MAX_BODY_H);
    v.body.setFrame(rect(PAD, y + 2.0, inner, body_h));
    y += 2.0 + body_h + 8.0;
    v.hint.setStringValue(&ns(&hint(card)));
    v.hint.setFrame(rect(PAD, y, inner, LINE_H));
    y + LINE_H + PAD - 2.0
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// The visible area of the screen under the pointer, else the main screen's.
fn screen_area(mtm: MainThreadMarker) -> Option<NSRect> {
    let mouse = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let under = screens.iter().find(|s| contains(s.frame(), mouse));
    under.or_else(|| NSScreen::mainScreen(mtm)).map(|s| s.visibleFrame())
}

fn contains(f: NSRect, p: NSPoint) -> bool {
    p.x >= f.origin.x
        && p.x < f.origin.x + f.size.width
        && p.y >= f.origin.y
        && p.y < f.origin.y + f.size.height
}

/// Show `card` (replacing any card showing) without taking focus.
pub fn show(card: &Card, placement: &Placement) {
    let mtm = super::mtm();
    let width = placement.width.clamp(240.0, 900.0);
    STATE.with_borrow_mut(|state| {
        let v = state.views.get_or_insert_with(|| make(mtm));
        let height = layout(v, card, width);
        let size = NSSize::new(width, height);
        let bounds = NSRect::new(NSPoint::ZERO, size);
        v.root.setFrame(bounds);
        v.effect.setFrame(bounds);
        let at = screen_area(mtm).map_or(NSPoint::ZERO, |a| origin(a, size, placement.corner));
        v.panel.setFrame_display(NSRect::new(at, size), true);
        v.panel.invalidateShadow();
        v.panel.orderFrontRegardless();
        state.link = card.link.map(str::to_string);
        if state.monitors.is_empty() {
            state.monitors = escape_monitors();
        }
    });
    let generation = GENERATION.get() + 1;
    GENERATION.set(generation);
    if placement.timeout_secs > 0.0 {
        timer::after(placement.timeout_secs, move || expire(generation));
    }
    if placement.sound
        && !card.pending
        && let Some(sound) = NSSound::soundNamed(&ns("Tink"))
    {
        sound.play();
    }
}

/// Hide the card, if it shows.
pub fn hide() {
    GENERATION.set(GENERATION.get() + 1);
    let monitors = STATE.with_borrow_mut(|state| {
        if let Some(v) = &state.views {
            v.panel.orderOut(None);
        }
        state.link = None;
        std::mem::take(&mut state.monitors)
    });
    for monitor in &monitors {
        // SAFETY: each monitor came from addGlobal/addLocalMonitorForEvents and is removed
        // once: `take` emptied the list.
        unsafe { NSEvent::removeMonitor(monitor) };
    }
}

/// The timeout of show number `generation`: hide, unless a later show or hide came first or
/// the pointer rests on the card.
fn expire(generation: u64) {
    if GENERATION.get() != generation {
        return;
    }
    let hovered = STATE.with_borrow(|s| {
        s.views.as_ref().is_some_and(|v| contains(v.panel.frame(), NSEvent::mouseLocation()))
    });
    if hovered {
        timer::after(HOVER_RECHECK, move || expire(generation));
    } else {
        hide();
    }
}

fn clicked() {
    let link = STATE.with_borrow(|s| s.link.clone());
    hide();
    if let Some(link) = link {
        workspace::open_url(&link);
    }
}

/// Escape anywhere hides the card: a global monitor for other apps' key events (needs
/// Accessibility; without it, only the click and the timeout dismiss) and a local one for
/// Flick's own.
fn escape_monitors() -> Vec<Retained<AnyObject>> {
    let global = RcBlock::new(|event: NonNull<NSEvent>| {
        // SAFETY: AppKit passes a valid event for the duration of the call.
        if unsafe { event.as_ref() }.keyCode() == ESCAPE {
            timer::after(0.0, hide);
        }
    });
    let local = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: as above.
        if unsafe { event.as_ref() }.keyCode() == ESCAPE {
            timer::after(0.0, hide);
        }
        event.as_ptr()
    });
    let global =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &global);
    // SAFETY: the block returns the event it was given, unchanged: a valid pointer.
    let local = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &local)
    };
    global.into_iter().chain(local).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_sit_inside_the_visible_area() {
        let area = NSRect::new(NSPoint::new(-1000.0, 50.0), NSSize::new(1000.0, 800.0));
        let size = NSSize::new(300.0, 100.0);
        let at = |c| {
            let p = origin(area, size, c);
            (p.x, p.y)
        };
        assert_eq!(at(Corner::TopRight), (-312.0, 738.0));
        assert_eq!(at(Corner::TopLeft), (-988.0, 738.0));
        assert_eq!(at(Corner::BottomRight), (-312.0, 62.0));
        assert_eq!(at(Corner::BottomLeft), (-988.0, 62.0));
        assert_eq!(at(Corner::Top), (-650.0, 738.0));
        assert_eq!(at(Corner::Bottom), (-650.0, 62.0));
    }

    #[test]
    fn the_hint_names_the_link_host() {
        let card =
            |pending, link| Card { header: "KOTA", time: "", context: "", body: "", link, pending };
        assert_eq!(hint(&card(false, None)), "Click or esc to dismiss");
        assert_eq!(
            hint(&card(false, Some("https://github.com/jayminwest/flick/pull/2"))),
            "Click to open github.com  ·  esc to dismiss"
        );
        assert_eq!(
            hint(&card(true, Some("https://x.y"))),
            "Waiting for a reply…  ·  esc to dismiss"
        );
        assert_eq!(host("mailto:a@b.c"), "mailto:a@b.c");
        assert_eq!(host("https://"), "https://");
    }
}
