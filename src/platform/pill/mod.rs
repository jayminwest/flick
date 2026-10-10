//! The recording pill: one small borderless, non-activating `NSPanel` near the bottom centre
//! of the screen under the pointer, for dictation. Separate from `platform::hud` (cards).
//!
//! - `show(Phase)` shows or redraws it: `Recording` (red dot, "Listening" and a level bar),
//!   `Transcribing`, `Result(text)` and `Error(text)`. A result or an error hides by itself
//!   (`layout::hide_after`); the others stay until the next `show` or `hide`.
//! - While recording, a self-rescheduling `timer::after(0.05)` reads the level from the
//!   phase's getter (0..=1) and redraws the bar. It stops when the phase changes, so an idle
//!   Flick runs no timer.
//! - Esc: a global key monitor (needs Accessibility) and a local one, installed only while
//!   the pill shows, hide it. During `Recording` or `Transcribing` Esc also calls the one
//!   `on_cancel` handler. Esc still reaches the app in front (a monitor cannot swallow it).
//! - The panel ignores the mouse and never becomes key, so focus stays where the text goes.

mod layout;

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSBox, NSBoxType, NSColor, NSEvent, NSEventMask, NSFont, NSLineBreakMode,
    NSPanel, NSScreen, NSTextField, NSTitlePosition, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::timer;
use layout::{Kind, Rect};

const ESCAPE: u16 = 53;
/// Above normal windows, like the HUD cards.
const LEVEL: isize = 25;
/// Seconds between level bar redraws while recording.
const LEVEL_SECS: f64 = 0.05;

/// What the pill shows.
#[cfg_attr(not(test), expect(dead_code, reason = "the dictation wiring (flick-a085) shows them"))]
#[derive(Clone, Copy, Debug)]
pub enum Phase<'a> {
    /// Recording; the getter returns the input level, 0..=1, read every 50 ms on the main
    /// thread (keep it to an atomic load).
    Recording(fn() -> f32),
    Transcribing,
    /// The inserted text (or a short note); hides after 1.5 s.
    Result(&'a str),
    /// Why nothing was inserted; hides after 4 s.
    Error(&'a str),
}

impl<'a> Phase<'a> {
    fn kind(self) -> Kind {
        match self {
            Phase::Recording(_) => Kind::Recording,
            Phase::Transcribing => Kind::Transcribing,
            Phase::Result(_) => Kind::Result,
            Phase::Error(_) => Kind::Error,
        }
    }

    /// The text of a result or an error, else empty.
    fn text(self) -> &'a str {
        match self {
            Phase::Result(t) | Phase::Error(t) => t,
            Phase::Recording(_) | Phase::Transcribing => "",
        }
    }
}

/// Where Esc during recording or transcribing goes: set once by `on_cancel`.
static ON_CANCEL: OnceLock<fn()> = OnceLock::new();

/// Send Esc presses during `Recording`/`Transcribing` to `handler`, on the main thread after
/// the pill hid and outside any pill state borrow. The first handler wins; it must only
/// queue work (post an event), never borrow app state. Tests must not call this.
#[expect(dead_code, reason = "the dictation wiring (flick-a085) cancels through it")]
pub fn on_cancel(handler: fn()) {
    let _ = ON_CANCEL.set(handler);
}

struct Views {
    panel: Retained<NSPanel>,
    effect: Retained<NSVisualEffectView>,
    dot: Retained<NSBox>,
    text: Retained<NSTextField>,
    track: Retained<NSBox>,
    fill: Retained<NSBox>,
}

#[derive(Default)]
struct State {
    views: Option<Views>,
    /// What shows now; `None` while hidden.
    kind: Option<Kind>,
    level: Option<fn() -> f32>,
    monitors: Vec<Retained<AnyObject>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
    /// Bumped by every show and hide; a timer from an older one does nothing.
    static EPOCH: Cell<u64> = const { Cell::new(0) };
}

fn next_epoch() -> u64 {
    EPOCH.set(EPOCH.get() + 1);
    EPOCH.get()
}

/// Show the pill in `phase`, or redraw it in place.
#[cfg_attr(not(test), expect(dead_code, reason = "the dictation wiring (flick-a085) shows it"))]
pub fn show(phase: Phase) {
    let mtm = super::mtm();
    let kind = phase.kind();
    let label = layout::label(kind, phase.text());
    let epoch = next_epoch();
    STATE.with_borrow_mut(|s| {
        let views = s.views.get_or_insert_with(|| make(mtm));
        draw(views, mtm, kind, &label);
        s.kind = Some(kind);
        s.level = match phase {
            Phase::Recording(level) => Some(level),
            _ => None,
        };
        if s.monitors.is_empty() {
            s.monitors = escape_monitors();
        }
    });
    if kind == Kind::Recording {
        tick(epoch);
    }
    if let Some(secs) = layout::hide_after(kind) {
        timer::after(secs, move || {
            if EPOCH.get() == epoch {
                hide();
            }
        });
    }
}

/// Hide the pill and remove its key monitors. Does nothing while hidden.
#[cfg_attr(not(test), expect(dead_code, reason = "the dictation wiring (flick-a085) hides it"))]
pub fn hide() {
    next_epoch();
    let monitors = STATE.with_borrow_mut(|s| {
        if let Some(v) = &s.views {
            v.panel.orderOut(None);
        }
        s.kind = None;
        s.level = None;
        std::mem::take(&mut s.monitors)
    });
    for monitor in &monitors {
        // SAFETY: each monitor came from addGlobal/addLocalMonitorForEvents and is removed
        // once: it was taken out of `State`.
        unsafe { NSEvent::removeMonitor(monitor) };
    }
}

/// Whether the pill shows.
#[cfg_attr(not(test), expect(dead_code, reason = "the dictation wiring (flick-a085)"))]
pub fn shown() -> bool {
    STATE.with_borrow(|s| s.kind.is_some())
}

/// Redraw the level bar and come back in 50 ms, while `epoch` is the live show.
fn tick(epoch: u64) {
    if EPOCH.get() != epoch {
        return;
    }
    let Some(level) = STATE.with_borrow(|s| s.level) else { return };
    let level = level();
    STATE.with_borrow(|s| {
        if let Some(v) = &s.views {
            set_fill(v, level);
        }
    });
    timer::after(LEVEL_SECS, move || tick(epoch));
}

/// Esc: hide, and tell the handler if work was in progress.
fn escape() {
    let Some(kind) = STATE.with_borrow(|s| s.kind) else { return };
    hide();
    if kind.cancellable()
        && let Some(handler) = ON_CANCEL.get()
    {
        handler();
    }
}

fn ns_rect(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}

fn draw(v: &Views, mtm: MainThreadMarker, kind: Kind, label: &str) {
    v.text.setStringValue(&NSString::from_str(label));
    let color = match kind {
        Kind::Error => NSColor::systemOrangeColor(),
        Kind::Result => NSColor::secondaryLabelColor(),
        Kind::Recording | Kind::Transcribing => NSColor::labelColor(),
    };
    v.text.setTextColor(Some(&color));
    let w = layout::width(kind, v.text.fittingSize().width.ceil());
    let parts = layout::parts(kind, w);
    v.effect.setFrame(ns_rect(Rect::new(0.0, 0.0, w, layout::H)));
    v.text.setFrame(ns_rect(parts.text));
    v.dot.setHidden(parts.dot.is_none());
    if let Some(dot) = parts.dot {
        v.dot.setFrame(ns_rect(dot));
    }
    v.track.setHidden(parts.track.is_none());
    v.fill.setHidden(parts.track.is_none());
    if let Some(track) = parts.track {
        v.track.setFrame(ns_rect(track));
        set_fill(v, 0.0);
    }
    if let Some(area) = screen_area(mtm) {
        let area = Rect::new(area.origin.x, area.origin.y, area.size.width, area.size.height);
        let frame = ns_rect(layout::frame(area, w));
        if v.panel.frame() != frame {
            v.panel.setFrame_display(frame, true);
            v.panel.invalidateShadow();
        }
    }
    v.panel.orderFrontRegardless();
}

fn set_fill(v: &Views, level: f32) {
    let mut f = v.track.frame();
    f.size.width = layout::fill(level, f.size.width);
    v.fill.setFrame(f);
}

/// The visible area of the screen under the pointer, else the main screen's.
fn screen_area(mtm: MainThreadMarker) -> Option<NSRect> {
    let p = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let under = screens.iter().find(|s| {
        let f = s.frame();
        p.x >= f.origin.x
            && p.x < f.origin.x + f.size.width
            && p.y >= f.origin.y
            && p.y < f.origin.y + f.size.height
    });
    under.or_else(|| NSScreen::mainScreen(mtm)).map(|s| s.visibleFrame())
}

/// A filled, borderless box: the dot, the bar's track and its fill.
fn filled(mtm: MainThreadMarker, color: &NSColor, radius: f64) -> Retained<NSBox> {
    // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
    let b: Retained<NSBox> = unsafe { msg_send![NSBox::alloc(mtm), initWithFrame: NSRect::ZERO] };
    b.setBoxType(NSBoxType::Custom);
    b.setTitlePosition(NSTitlePosition::NoTitle);
    b.setBorderWidth(0.0);
    b.setCornerRadius(radius);
    b.setFillColor(color);
    b
}

/// The hidden pill and its views.
fn make(mtm: MainThreadMarker) -> Views {
    let rect = NSRect::new(NSPoint::ZERO, NSSize::new(200.0, layout::H));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: NSPanel's designated initializer, with argument types matching its signature.
    let panel: Retained<NSPanel> = unsafe {
        msg_send![NSPanel::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setIgnoresMouseEvents(true);
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
    // SAFETY: `State` holds the panel for the life of the process and never closes it.
    unsafe { panel.setReleasedWhenClosed(false) };

    let effect = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), rect);
    effect.setMaterial(NSVisualEffectMaterial::HUDWindow);
    effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect.setState(NSVisualEffectState::Active);
    effect.setWantsLayer(true);
    if let Some(layer) = effect.layer() {
        layer.setCornerRadius(layout::H / 2.0);
        layer.setMasksToBounds(true);
    }
    let text = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    text.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    text.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    let dot = filled(mtm, &NSColor::systemRedColor(), 4.0);
    let track = filled(mtm, &NSColor::quaternaryLabelColor(), 2.0);
    let fill = filled(mtm, &NSColor::labelColor(), 2.0);
    for view in [&*dot as &NSView, &text, &track, &fill] {
        effect.addSubview(view);
    }
    panel.setContentView(Some(&effect));
    Views { panel, effect, dot, text, track, fill }
}

/// Escape anywhere: a global monitor for other apps' key events (needs Accessibility) and a
/// local one for Flick's own. Both defer to the next run loop turn.
fn escape_monitors() -> Vec<Retained<AnyObject>> {
    let global = RcBlock::new(|event: NonNull<NSEvent>| {
        // SAFETY: AppKit passes a valid event for the duration of the call.
        if unsafe { event.as_ref() }.keyCode() == ESCAPE {
            timer::after(0.0, escape);
        }
    });
    let local = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: as above.
        if unsafe { event.as_ref() }.keyCode() == ESCAPE {
            timer::after(0.0, escape);
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

    fn level() -> f32 {
        0.5
    }

    #[test]
    fn phases_map_to_kinds() {
        assert_eq!(Phase::Recording(level).kind(), Kind::Recording);
        assert_eq!(Phase::Transcribing.kind(), Kind::Transcribing);
        assert_eq!(Phase::Result("hi").kind(), Kind::Result);
        assert_eq!(Phase::Error("no").kind(), Kind::Error);
        assert_eq!(Phase::Result("hi").text(), "hi");
        assert_eq!(Phase::Error("no").text(), "no");
        assert_eq!(Phase::Recording(level).text(), "");
        assert_eq!(Phase::Transcribing.text(), "");
        // Not called: they need the main thread and draw a real panel.
        let _: fn(Phase) = show;
        let _: fn() = hide;
        let _: fn() -> bool = shown;
    }
}
