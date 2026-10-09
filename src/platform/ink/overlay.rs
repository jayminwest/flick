//! Drawing on the screen: one borderless, click-through panel per display above every app
//! (menu bar, Dock and full-screen spaces included), each with its own `FlickInkView`, and a
//! cursor halo that follows the pointer.
//!
//! In draw mode the panels take the mouse and become key without activating Flick, so the
//! canvas keys work (tools, colors, undo, Delete); Return ends drawing and keeps the shapes,
//! Esc ends drawing and clears them. Out of draw mode the shapes stay, click-through, until
//! Esc or `clear`. With `Style::fade_secs` set, each finished shape goes after that long.
//! Panels exist only while drawing or while shapes remain, so an idle overlay costs nothing.
//!
//! The halo is a small panel moved by `NSEvent` global and local mouse monitors (no
//! Accessibility permission needed, no timer); the monitors exist only while the halo is on.
//! Everything here runs on the main thread; callbacks run while `AppKit` is mid-event, so
//! they never hold the overlay state across a call into `AppKit` that could re-enter.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSEvent, NSEventMask, NSEventType, NSPanel,
    NSResponder, NSScreen, NSScreenSaverWindowLevel, NSView, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize};

use super::model::{Command, Style};
use super::view::{FlickInkView, now, ns_color};
use crate::platform::{Rect, mtm, timer};

/// The halo's ring width, and how long a click pulse shows.
const RING: f64 = 3.0;
const PULSE_SECS: f64 = 0.2;
/// Opacity of the filled click pulse, relative to the halo color.
const PULSE_ALPHA: f32 = 0.35;

/// The cursor highlight: a ring of `radius` points around the pointer in `color` (RGBA).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Halo {
    pub color: [f32; 4],
    pub radius: f64,
}

define_class!(
    // Borderless panels refuse key status by default; draw mode needs it for keys.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickInkOverlayPanel"]
    struct OverlayPanel;

    impl OverlayPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            DRAWING.get()
        }
    }
);

/// What a `HaloView` draws.
struct Ring {
    color: Cell<[f32; 4]>,
    pulse: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickInkHaloView"]
    #[ivars = Ring]
    struct HaloView;

    impl HaloView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let ring = self.ivars();
            let [r, g, b, a] = ring.color.get();
            let frame = self.bounds();
            let inset = NSRect::new(
                NSPoint::new(frame.origin.x + RING, frame.origin.y + RING),
                NSSize::new(frame.size.width - 2.0 * RING, frame.size.height - 2.0 * RING),
            );
            let oval = NSBezierPath::bezierPathWithOvalInRect(inset);
            if ring.pulse.get() {
                ns_color([r, g, b, a * PULSE_ALPHA]).setFill();
                oval.fill();
            }
            oval.setLineWidth(RING);
            ns_color([r, g, b, a]).setStroke();
            oval.stroke();
        }
    }
);

/// One display's panel and canvas; `frame` is the screen's frame it covers.
struct Screen {
    panel: Retained<OverlayPanel>,
    view: Retained<FlickInkView>,
    frame: Rect,
}

/// The halo's panel and view and the mouse monitors that move it.
struct Cursor {
    panel: Retained<OverlayPanel>,
    view: Retained<HaloView>,
    radius: f64,
    monitors: Vec<Retained<AnyObject>>,
}

#[derive(Default)]
struct Overlay {
    screens: Vec<Screen>,
    style: Style,
    cursor: Option<Cursor>,
}

thread_local! {
    static STATE: RefCell<Overlay> = RefCell::new(Overlay::default());
    /// Draw mode, apart from `STATE` so `canBecomeKeyWindow` never contends for it.
    static DRAWING: Cell<bool> = const { Cell::new(false) };
}

/// Turn draw mode on (the panels take the mouse and keys, without activating Flick) or off
/// (click-through; shapes stay until Esc or `clear`). `style` sets colors, width and fade.
#[expect(
    dead_code,
    reason = "wired in flick-abc0 step 6 (flick-2759); needs the main thread, so no test calls it"
)]
pub fn set_drawing(on: bool, style: &Style) {
    STATE.with_borrow_mut(|s| s.style = style.clone());
    DRAWING.set(on);
    if on {
        let screens = build(take_screens());
        let mouse = NSEvent::mouseLocation();
        for screen in &screens {
            screen.view.set_style(style.clone());
            screen.panel.setIgnoresMouseEvents(false);
            screen.panel.orderFrontRegardless();
            if contains(screen.frame, mouse) {
                screen.panel.makeKeyWindow();
                screen.panel.makeFirstResponder(Some(&screen.view));
            }
        }
        STATE.with_borrow_mut(|s| s.screens = screens);
    } else {
        stop();
    }
}

/// Whether draw mode is on.
#[expect(dead_code, reason = "wired in flick-abc0 step 6 (flick-2759)")]
pub fn drawing() -> bool {
    DRAWING.get()
}

/// Whether any shape is on the screen.
#[expect(dead_code, reason = "wired in flick-abc0 step 6 (flick-2759)")]
pub fn has_shapes() -> bool {
    STATE.with_borrow(|s| s.screens.iter().any(|screen| !screen.view.is_empty()))
}

/// Drop every shape on every display.
#[expect(dead_code, reason = "wired in flick-abc0 step 6 (flick-2759)")]
pub fn clear() {
    for view in views() {
        view.clear();
    }
    tidy();
}

/// Show `halo` around the pointer, or hide it with `None`.
#[expect(
    dead_code,
    reason = "wired in flick-abc0 step 6 (flick-2759); installs global monitors, so no test calls it"
)]
pub fn set_cursor(halo: Option<Halo>) {
    if let Some(cursor) = STATE.with_borrow_mut(|s| s.cursor.take()) {
        for monitor in &cursor.monitors {
            // SAFETY: each monitor came from addGlobal/addLocalMonitorForEvents and is
            // removed once.
            unsafe { NSEvent::removeMonitor(monitor) };
        }
        cursor.panel.orderOut(None);
    }
    let Some(halo) = halo else { return };
    let mtm = mtm();
    let side = halo_side(halo.radius);
    let frame = NSRect::new(NSPoint::ZERO, NSSize::new(side, side));
    let panel = panel(mtm, frame, NSScreenSaverWindowLevel + 1);
    panel.setIgnoresMouseEvents(true);
    let view = HaloView::alloc(mtm)
        .set_ivars(Ring { color: Cell::new(halo.color), pulse: Cell::new(false) });
    // SAFETY: NSView's designated initializer, with the argument type of its signature.
    let view: Retained<HaloView> = unsafe { msg_send![super(view), initWithFrame: frame] };
    panel.setContentView(Some(&view));
    let cursor = Cursor { panel, view, radius: halo.radius, monitors: monitors() };
    place(&cursor, false);
    cursor.panel.orderFrontRegardless();
    STATE.with_borrow_mut(|s| s.cursor = Some(cursor));
}

/// Fit the panels to the current displays (after a display is added, removed or moved).
/// Displays that kept their frame keep their shapes.
#[expect(dead_code, reason = "wired in flick-abc0 step 6 (flick-2759)")]
pub fn relayout() {
    if STATE.with_borrow(|s| s.screens.is_empty()) {
        return;
    }
    let screens = build(take_screens());
    let style = STATE.with_borrow(|s| s.style.clone());
    let on = DRAWING.get();
    for screen in &screens {
        screen.view.set_style(style.clone());
        screen.panel.setIgnoresMouseEvents(!on);
        screen.panel.orderFrontRegardless();
    }
    STATE.with_borrow_mut(|s| s.screens = screens);
    tidy();
}

/// End draw mode: click-through, and give up key status (ordering out resigns it).
fn stop() {
    for screen in
        STATE.with_borrow(|s| s.screens.iter().map(|x| x.panel.clone()).collect::<Vec<_>>())
    {
        screen.setIgnoresMouseEvents(true);
        if screen.isKeyWindow() {
            screen.orderOut(None);
            screen.orderFrontRegardless();
        }
    }
    tidy();
}

/// Out of draw mode with nothing left to show, release the panels. They go on the next run
/// loop pass: this may run inside one of their views' event methods.
fn tidy() {
    if DRAWING.get() || views().iter().any(|v| !v.is_empty()) {
        return;
    }
    let screens = take_screens();
    for screen in &screens {
        screen.panel.orderOut(None);
    }
    release(screens);
}

/// The panels for the current displays, reusing those in `old` whose frame still matches.
/// The rest of `old` is ordered out and released.
fn build(old: Vec<Screen>) -> Vec<Screen> {
    let mtm = mtm();
    let frames: Vec<Rect> = NSScreen::screens(mtm).iter().map(|s| rect(s.frame())).collect();
    let old_frames: Vec<Rect> = old.iter().map(|s| s.frame).collect();
    let reuse = matches(&old_frames, &frames);
    let mut old: Vec<Option<Screen>> = old.into_iter().map(Some).collect();
    let screens = frames
        .into_iter()
        .zip(reuse)
        .map(|(frame, slot)| {
            slot.and_then(|i| old.get_mut(i)?.take()).unwrap_or_else(|| new_screen(mtm, frame))
        })
        .collect();
    let stale: Vec<Screen> = old.into_iter().flatten().collect();
    for screen in &stale {
        screen.panel.orderOut(None);
    }
    release(stale);
    screens
}

fn new_screen(mtm: MainThreadMarker, frame: Rect) -> Screen {
    let ns = ns_rect(frame);
    let panel = panel(mtm, ns, NSScreenSaverWindowLevel);
    let bounds = NSRect::new(NSPoint::ZERO, ns.size);
    let style = STATE.with_borrow(|s| s.style.clone());
    let view = FlickInkView::new(mtm, bounds, style, Some(command));
    view.set_on_stroke(Some(stroked));
    panel.setContentView(Some(&view));
    Screen { panel, view, frame }
}

/// A clear, borderless, non-activating panel at `level` on every space.
fn panel(mtm: MainThreadMarker, frame: NSRect, level: isize) -> Retained<OverlayPanel> {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: NSPanel's designated initializer, with argument types matching its signature.
    let panel: Retained<OverlayPanel> = unsafe {
        msg_send![OverlayPanel::alloc(mtm), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    // SAFETY: the overlay holds each panel until after it is ordered out for good.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setLevel(level);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setHidesOnDeactivate(false);
    panel.setFrame_display(frame, false);
    panel
}

/// The canvas's Return, cmd+C, cmd+S and Esc.
fn command(cmd: Command) {
    match key_action(cmd) {
        Some(Action::Stop) => {
            DRAWING.set(false);
            stop();
        }
        Some(Action::StopAndClear) => {
            DRAWING.set(false);
            for view in views() {
                view.clear();
            }
            stop();
        }
        None => {}
    }
}

/// What a canvas command does to draw mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Stop,
    StopAndClear,
}

fn key_action(cmd: Command) -> Option<Action> {
    match cmd {
        Command::Done => Some(Action::Stop),
        Command::Cancel => Some(Action::StopAndClear),
        Command::Copy
        | Command::Save
        | Command::Tool(_)
        | Command::Color(_)
        | Command::Undo
        | Command::Redo
        | Command::Clear => None,
    }
}

/// A shape was finished: with fade on, expire it once it is old enough.
fn stroked() {
    let fade = STATE.with_borrow(|s| s.style.fade_secs);
    if fade > 0.0 {
        timer::after(f64::from(fade), move || {
            for view in views() {
                view.expire(now(), fade);
            }
            tidy();
        });
    }
}

/// Global monitors see events sent to other apps; local ones see Flick's own (the
/// launcher, the overlay in draw mode). Both move the halo and pulse it on clicks.
fn monitors() -> Vec<Retained<AnyObject>> {
    let mask = NSEventMask::MouseMoved
        | NSEventMask::LeftMouseDragged
        | NSEventMask::RightMouseDragged
        | NSEventMask::OtherMouseDragged
        | NSEventMask::LeftMouseDown
        | NSEventMask::RightMouseDown
        | NSEventMask::OtherMouseDown;
    let global = RcBlock::new(|event: NonNull<NSEvent>| {
        // SAFETY: AppKit passes a valid event for the duration of the call.
        moved(unsafe { event.as_ref() });
    });
    let local = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: as above.
        moved(unsafe { event.as_ref() });
        event.as_ptr()
    });
    let global = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &global);
    // SAFETY: the block returns the event it was given, unchanged: a valid pointer.
    let local = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local) };
    global.into_iter().chain(local).collect()
}

fn moved(event: &NSEvent) {
    let down = matches!(
        event.r#type(),
        NSEventType::LeftMouseDown | NSEventType::RightMouseDown | NSEventType::OtherMouseDown
    );
    let cursor = STATE.with(|s| s.try_borrow().ok()?.cursor.as_ref().map(clone_cursor));
    if let Some((panel, view, radius)) = cursor {
        place_at(&panel, &view, radius, down);
    }
}

fn clone_cursor(c: &Cursor) -> (Retained<OverlayPanel>, Retained<HaloView>, f64) {
    (c.panel.clone(), c.view.clone(), c.radius)
}

fn place(cursor: &Cursor, pulse: bool) {
    place_at(&cursor.panel, &cursor.view, cursor.radius, pulse);
}

/// Center the halo on the pointer; a click shows the pulse for a moment.
fn place_at(panel: &OverlayPanel, view: &HaloView, radius: f64, pulse: bool) {
    let r = halo_frame(NSEvent::mouseLocation(), radius);
    panel.setFrameOrigin(NSPoint::new(r.x, r.y));
    if pulse {
        view.ivars().pulse.set(true);
        view.setNeedsDisplay(true);
        let view = view.retain();
        timer::after(PULSE_SECS, move || {
            view.ivars().pulse.set(false);
            view.setNeedsDisplay(true);
        });
    }
}

/// The views of every display, cloned so no borrow is held while they run.
fn views() -> Vec<Retained<FlickInkView>> {
    STATE.with_borrow(|s| s.screens.iter().map(|x| x.view.clone()).collect())
}

fn take_screens() -> Vec<Screen> {
    STATE.with_borrow_mut(|s| std::mem::take(&mut s.screens))
}

/// Drop `screens` on the next run loop pass.
fn release(screens: Vec<Screen>) {
    if screens.is_empty() {
        return;
    }
    let slot = Cell::new(Some(screens));
    timer::after(0.0, move || drop(slot.take()));
}

/// For each of `new`, the index in `old` of a screen with the same frame, each used once.
fn matches(old: &[Rect], new: &[Rect]) -> Vec<Option<usize>> {
    let mut used = vec![false; old.len()];
    new.iter()
        .map(|frame| {
            let i = old.iter().zip(&used).position(|(o, &u)| !u && o == frame)?;
            used[i] = true;
            Some(i)
        })
        .collect()
}

/// The halo panel's side: the ring's diameter plus its stroke.
fn halo_side(radius: f64) -> f64 {
    2.0 * (radius.max(1.0) + RING)
}

/// The halo panel's frame (screen coordinates, origin bottom-left) centered on `mouse`.
fn halo_frame(mouse: NSPoint, radius: f64) -> Rect {
    let side = halo_side(radius);
    Rect { x: mouse.x - side / 2.0, y: mouse.y - side / 2.0, w: side, h: side }
}

fn contains(r: Rect, p: NSPoint) -> bool {
    p.x >= r.x && p.x < r.x + r.w && p.y >= r.y && p.y < r.y + r.h
}

fn rect(f: NSRect) -> Rect {
    Rect { x: f.origin.x, y: f.origin.y, w: f.size.width, h: f.size.height }
}

fn ns_rect(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}

#[cfg(test)]
mod tests;
