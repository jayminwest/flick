//! The tap thread: create the tap, run its run loop, keep it enabled, call the handler.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use super::{
    CFRelease, FIELD_SOURCE_USER_DATA, Handler, INJECTED_MARK, Kind, Rearm, Shared, Status,
    TapEvent, Verdict,
};

type CFTypeRef = *const c_void;
type CGEventRef = *mut c_void;
type TapCallback = extern "C" fn(CFTypeRef, u32, CGEventRef, *mut c_void) -> CGEventRef;

/// Seconds between health checks of the tap.
const REARM_SECS: f64 = 5.0;
/// `kCGSessionEventTap`, `kCGHeadInsertEventTap`, `kCGEventTapOptionDefault` (active).
const SESSION_TAP: u32 = 1;
const HEAD_INSERT: u32 = 0;
const TAP_ACTIVE: u32 = 0;
/// `CGEventType` values.
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
/// `CGEventField` values.
const FIELD_AUTOREPEAT: u32 = 8;
const FIELD_KEYCODE: u32 = 9;
/// `kCFRunLoopRunStopped`.
const RUN_STOPPED: i32 = 2;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFTypeRef;
    fn CGEventTapEnable(tap: CFTypeRef, enable: bool);
    fn CGEventTapIsEnabled(tap: CFTypeRef) -> bool;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: CFTypeRef;
    fn CFMachPortCreateRunLoopSource(alloc: CFTypeRef, port: CFTypeRef, order: isize) -> CFTypeRef;
    fn CFMachPortInvalidate(port: CFTypeRef);
    fn CFRunLoopGetCurrent() -> CFTypeRef;
    fn CFRunLoopAddSource(rl: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
    fn CFRunLoopRemoveSource(rl: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
    fn CFRunLoopRunInMode(mode: CFTypeRef, seconds: f64, return_after_source: bool) -> i32;
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// What the callback reaches through its user-info pointer: shared, never `&mut`, because
/// the callback runs inside `run`'s run loop. Lives on the tap thread's stack until the tap
/// is gone.
struct Context<'a> {
    shared: &'a Shared,
    handler: RefCell<Handler>,
    on_rearm: RefCell<Rearm>,
    /// The tap (a `CFMachPort`) and its run loop source, null while there is none.
    port: Cell<CFTypeRef>,
    source: Cell<CFTypeRef>,
    /// This thread's run loop.
    run_loop: CFTypeRef,
}

/// The tap thread: keep a tap alive until `stop`.
pub(super) fn run(shared: &Shared, handler: Handler, on_rearm: Rearm) {
    // SAFETY: plain C call; it returns the current thread's run loop, not retained.
    let current = unsafe { CFRunLoopGetCurrent() };
    // SAFETY: `current` is live while this thread runs; `Shared` releases this +1.
    let run_loop = unsafe { CFRetain(current) };
    *shared.run_loop.lock().unwrap_or_else(PoisonError::into_inner) = run_loop as usize;
    let cx = Context {
        shared,
        handler: RefCell::new(handler),
        on_rearm: RefCell::new(on_rearm),
        port: Cell::new(ptr::null()),
        source: Cell::new(ptr::null()),
        run_loop,
    };
    let mut created_before = false;
    while !shared.stop.load(Ordering::SeqCst) {
        if cx.port.get().is_null() {
            if !create(&cx) {
                shared.set_status(Status::NoPermission);
                thread::park_timeout(Duration::from_secs_f64(REARM_SECS));
                continue;
            }
            shared.set_status(Status::Running);
            if created_before {
                rearmed(&cx);
            }
            created_before = true;
        }
        // SAFETY: runs this thread's own run loop; the mode is a framework constant.
        let why = unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, REARM_SECS, false) };
        if why != RUN_STOPPED && !shared.stop.load(Ordering::SeqCst) {
            check(&cx);
        }
    }
    destroy(&cx);
    shared.set_status(Status::Off);
}

/// Whether Flick has Accessibility, which an active tap needs. Never prompts.
fn trusted() -> bool {
    // SAFETY: plain C call without arguments.
    unsafe { AXIsProcessTrusted() }
}

/// Create the tap and add it to the thread's run loop. False without permission or on failure.
fn create(cx: &Context) -> bool {
    if !trusted() {
        return false;
    }
    let mask = (1u64 << KEY_DOWN) | (1 << KEY_UP) | (1 << FLAGS_CHANGED);
    let info = ptr::from_ref(cx).cast_mut().cast::<c_void>();
    // SAFETY: valid enum values and a plain `extern "C"` callback; `info` points at the
    // `Context` on the tap thread's stack, which outlives the tap (`destroy` runs first).
    let port =
        unsafe { CGEventTapCreate(SESSION_TAP, HEAD_INSERT, TAP_ACTIVE, mask, callback, info) };
    if port.is_null() {
        return false;
    }
    // SAFETY: `port` is a live CFMachPort; the result is a +1 reference or null.
    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), port, 0) };
    if source.is_null() {
        // SAFETY: `port` is a live tap we own (+1), used no more.
        unsafe { CFMachPortInvalidate(port) };
        // SAFETY: as above.
        unsafe { CFRelease(port) };
        return false;
    }
    // SAFETY: `run_loop` is this thread's run loop, `source` a live source, the mode a
    // constant.
    unsafe { CFRunLoopAddSource(cx.run_loop, source, kCFRunLoopDefaultMode) };
    cx.port.set(port);
    cx.source.set(source);
    // SAFETY: `port` is a live tap.
    unsafe { CGEventTapEnable(port, true) };
    true
}

/// Remove the tap from the thread's run loop and release it.
fn destroy(cx: &Context) {
    let source = cx.source.replace(ptr::null());
    if !source.is_null() {
        // SAFETY: `source` is the live source `create` added to this run loop; we own its +1.
        unsafe { CFRunLoopRemoveSource(cx.run_loop, source, kCFRunLoopDefaultMode) };
        // SAFETY: as above, used no more.
        unsafe { CFRelease(source) };
    }
    let port = cx.port.replace(ptr::null());
    if !port.is_null() {
        // SAFETY: `port` is the live tap `create` made; we own its +1, used no more.
        unsafe { CFMachPortInvalidate(port) };
        // SAFETY: as above.
        unsafe { CFRelease(port) };
    }
}

/// Health check: re-enable a tap macOS disabled; drop it if permission is gone, so it is
/// created again after a new grant.
fn check(cx: &Context) {
    let port = cx.port.get();
    // SAFETY: `port` is a live tap (`run` calls this only while it has one).
    if unsafe { CGEventTapIsEnabled(port) } {
        return;
    }
    if !trusted() {
        destroy(cx);
        cx.shared.set_status(Status::NoPermission);
        return;
    }
    // SAFETY: as above.
    unsafe { CGEventTapEnable(port, true) };
    // SAFETY: as above.
    if unsafe { CGEventTapIsEnabled(port) } {
        cx.shared.set_status(Status::Running);
        rearmed(cx);
    } else {
        cx.shared.set_status(Status::Disabled);
    }
}

/// Run the re-arm hook (unless it is already running); a panic in it is logged and stops
/// there.
fn rearmed(cx: &Context) {
    let Ok(mut hook) = cx.on_rearm.try_borrow_mut() else { return };
    if catch_unwind(AssertUnwindSafe(|| (*hook)())).is_err() {
        eprintln!("flick: keytap re-arm hook panicked");
    }
}

/// The `Kind` for a `CGEventType`, or None for event types the handler never sees.
fn kind(event_type: u32) -> Option<Kind> {
    match event_type {
        KEY_DOWN => Some(Kind::Down),
        KEY_UP => Some(Kind::Up),
        FLAGS_CHANGED => Some(Kind::FlagsChanged),
        _ => None,
    }
}

/// The tap callback. Never unwinds: a panicking or busy handler passes the event.
extern "C" fn callback(
    _proxy: CFTypeRef,
    event_type: u32,
    event: CGEventRef,
    info: *mut c_void,
) -> CGEventRef {
    // SAFETY: `info` is the `Context` that `create` registered. It outlives the tap, and
    // the tap calls back only on the tap thread; all mutation goes through its cells.
    let cx = unsafe { &*info.cast::<Context>() };
    if matches!(event_type, DISABLED_BY_TIMEOUT | DISABLED_BY_USER_INPUT) {
        let port = cx.port.get();
        if !port.is_null() {
            // SAFETY: `port` is the live tap this callback belongs to.
            unsafe { CGEventTapEnable(port, true) };
            rearmed(cx);
        }
        return event;
    }
    let Some(kind) = kind(event_type) else { return event };
    if event.is_null() || cx.shared.stop.load(Ordering::SeqCst) {
        return event;
    }
    // SAFETY: `event` is the live, non-null CGEvent this callback was given; the field ids
    // are valid `CGEventField`s.
    let field = |f| unsafe { CGEventGetIntegerValueField(event, f) };
    let tap_event = TapEvent {
        kind,
        keycode: field(FIELD_KEYCODE) as u16,
        // SAFETY: as above.
        flags: unsafe { CGEventGetFlags(event) },
        autorepeat: field(FIELD_AUTOREPEAT) != 0,
        injected: field(FIELD_SOURCE_USER_DATA) == INJECTED_MARK,
    };
    let Ok(mut handler) = cx.handler.try_borrow_mut() else { return event };
    match catch_unwind(AssertUnwindSafe(|| (*handler)(tap_event))) {
        Ok(Verdict::Pass) => event,
        Ok(Verdict::Drop) => ptr::null_mut(),
        Ok(Verdict::SetFlags(flags)) => {
            // SAFETY: as above; a tap may change the event it returns.
            unsafe { super::CGEventSetFlags(event, flags) };
            event
        }
        Err(_) => {
            eprintln!("flick: keytap handler panicked");
            event
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_types_map_to_kinds() {
        assert_eq!(kind(KEY_DOWN), Some(Kind::Down));
        assert_eq!(kind(KEY_UP), Some(Kind::Up));
        assert_eq!(kind(FLAGS_CHANGED), Some(Kind::FlagsChanged));
        assert_eq!(kind(DISABLED_BY_TIMEOUT), None);
        assert_eq!(kind(1), None);
    }

    #[test]
    fn trust_check_does_not_prompt() {
        let _ = trusted();
    }
}
