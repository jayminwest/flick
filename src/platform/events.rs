//! Platform observers that turn macOS notifications into `core::Event`s, and the main-queue
//! hand-off for events and work raised on other threads.
//!
//! Observers run on the main thread and call the sink synchronously. macOS has no
//! notification for pasteboard writes or user idle time, so one half-second timer polls the
//! pasteboard's change count (and, every few seconds, the idle time); nothing else polls.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock, PoisonError};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSWorkspace,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidWakeNotification,
    NSWorkspaceSessionDidBecomeActiveNotification, NSWorkspaceSessionDidResignActiveNotification,
    NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSNotificationName,
    NSObjectProtocol, NSString,
};

use super::{pasteboard, timer, workspace};
use crate::core::Event;

/// Seconds between pasteboard checks.
const POLL_SECS: f64 = 0.5;
/// Check idle time every this many polls.
const IDLE_EVERY: u32 = 10;
/// Seconds without input before `Event::Idle`.
const IDLE_AFTER_SECS: f64 = 60.0;

/// Where events go: set once by `start`, called on the main thread only.
static SINK: OnceLock<fn(Event)> = OnceLock::new();
/// Events posted from any thread, waiting for the main queue to drain them.
static POSTED: Mutex<Vec<Event>> = Mutex::new(Vec::new());

thread_local! {
    /// Notification observers; dropping one ends its registration.
    static OBSERVERS: RefCell<Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>> =
        const { RefCell::new(Vec::new()) };
}

#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

unsafe extern "C" {
    /// The main dispatch queue (`dispatch_get_main_queue()` is a macro over this symbol).
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(
        queue: *const DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
}

/// `kCGEventSourceStateCombinedSessionState`, `kCGAnyInputEventType`.
const COMBINED_SESSION: i32 = 0;
const ANY_INPUT: u32 = !0;

/// Start every observer; each event goes to `sink` on the main thread. Call once.
pub fn start(sink: fn(Event)) {
    if SINK.set(sink).is_err() {
        return;
    }
    let ws = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: notification names are immutable framework constants, set at load time.
    let (activated, wake, screens) = unsafe {
        (
            NSWorkspaceDidActivateApplicationNotification,
            NSWorkspaceDidWakeNotification,
            NSApplicationDidChangeScreenParametersNotification,
        )
    };
    observe(&ws, activated, || workspace::frontmost_pid().map(|pid| Event::AppActivated { pid }));
    observe(&ws, wake, || Some(Event::Wake));
    observe(&NSNotificationCenter::defaultCenter(), screens, || Some(Event::DisplaysChanged));

    let count = Cell::new(pasteboard::change_count());
    let polls = Cell::new(0u32);
    let idle = Cell::new(false);
    timer::every(POLL_SECS, move || {
        let now = pasteboard::change_count();
        if count.replace(now) != now {
            sink(Event::PasteboardChanged);
        }
        polls.set((polls.get() + 1) % IDLE_EVERY);
        if polls.get() == 0 {
            let secs = idle_secs();
            if !idle.get() && secs >= IDLE_AFTER_SECS {
                idle.set(true);
                sink(Event::Idle { secs: secs as u64 });
            } else if idle.get() && secs < IDLE_AFTER_SECS {
                idle.set(false);
                sink(Event::Active);
            }
        }
    });
}

/// Seconds since the last keyboard or mouse input in this login session.
fn idle_secs() -> f64 {
    // SAFETY: plain C call with valid enum values; it only reads input timestamps.
    unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION, ANY_INPUT) }
}

/// Send `event(…)` to the sink on each `name` notification from `center`.
fn observe(
    center: &NSNotificationCenter,
    name: &NSNotificationName,
    event: impl Fn() -> Option<Event> + 'static,
) {
    observe_with(center, name, move || {
        if let (Some(event), Some(sink)) = (event(), SINK.get()) {
            sink(event);
        }
    });
}

/// A change in whether this login session is in use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-a30b"))]
pub enum SessionChange {
    /// The machine is about to sleep.
    Sleep,
    /// The screen locked, or another user's session took over (fast user switching).
    Locked,
    /// The screen unlocked, or this session became active again.
    Unlocked,
}

/// Call `on_change` on the main thread on sleep, screen lock/unlock and fast user switching.
/// Nothing is observed until this is called.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-a30b"))]
pub fn on_session(on_change: fn(SessionChange)) {
    let ws = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: notification names are immutable framework constants, set at load time.
    let (sleep, resign, become_active) = unsafe {
        (
            NSWorkspaceWillSleepNotification,
            NSWorkspaceSessionDidResignActiveNotification,
            NSWorkspaceSessionDidBecomeActiveNotification,
        )
    };
    observe_with(&ws, sleep, move || on_change(SessionChange::Sleep));
    observe_with(&ws, resign, move || on_change(SessionChange::Locked));
    observe_with(&ws, become_active, move || on_change(SessionChange::Unlocked));
    // Screen lock has no public notification name; loginwindow posts these on the
    // distributed center, delivered on the main thread.
    let distributed = NSDistributedNotificationCenter::defaultCenter();
    let locked = NSString::from_str(SCREEN_LOCKED);
    let unlocked = NSString::from_str(SCREEN_UNLOCKED);
    observe_with(&distributed, &locked, move || on_change(SessionChange::Locked));
    observe_with(&distributed, &unlocked, move || on_change(SessionChange::Unlocked));
}

const SCREEN_LOCKED: &str = "com.apple.screenIsLocked";
const SCREEN_UNLOCKED: &str = "com.apple.screenIsUnlocked";

/// Call `f` on each `name` notification from `center`, on the posting thread.
fn observe_with(center: &NSNotificationCenter, name: &NSNotificationName, f: impl Fn() + 'static) {
    let block = RcBlock::new(move |_n: NonNull<NSNotification>| f());
    // SAFETY: the block is 'static, takes the `NSNotification` argument the center passes,
    // and runs on the posting (main) thread; the observer is kept below, which keeps the
    // registration.
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    OBSERVERS.with(|o| o.borrow_mut().push(observer));
}

/// Queue `event` for the sink from any thread; the main queue delivers it on its next turn.
/// Events posted before `start` are dropped.
pub fn post(event: Event) {
    let mut posted = POSTED.lock().unwrap_or_else(PoisonError::into_inner);
    posted.push(event);
    if posted.len() == 1 {
        // SAFETY: `_dispatch_main_q` is libdispatch's static main queue; `drain` ignores the
        // null context and is a plain function, so nothing outlives the call.
        unsafe { dispatch_async_f(&raw const _dispatch_main_q, std::ptr::null_mut(), drain) };
    }
}

/// The main dispatch queue, for other dispatch sources in `platform`.
pub(super) fn main_queue() -> *const c_void {
    (&raw const _dispatch_main_q).cast()
}

/// Main queue: hand every posted event to the sink, oldest first.
extern "C" fn drain(_context: *mut c_void) {
    let events = std::mem::take(&mut *POSTED.lock().unwrap_or_else(PoisonError::into_inner));
    if let Some(sink) = SINK.get() {
        events.into_iter().for_each(sink);
    }
}

/// Work queued by `on_main`.
type Job = Box<dyn FnOnce() + Send>;

/// Run `job` on the main queue's next turn, from any thread. A panic in `job` is logged and
/// stops there: it never unwinds into libdispatch.
pub fn on_main(job: impl FnOnce() + Send + 'static) {
    let context = Box::into_raw(Box::new(Box::new(job) as Job)).cast::<c_void>();
    // SAFETY: as in `post`; `run_job` takes back ownership of `context`, a leaked `Box<Job>`,
    // exactly once.
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, context, run_job) };
}

/// Main queue: run one job from `on_main`.
extern "C" fn run_job(context: *mut c_void) {
    // SAFETY: `context` is the `Box<Job>` that `on_main` leaked, delivered once.
    let job = unsafe { Box::from_raw(context.cast::<Job>()) };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
        eprintln!("flick: main-queue job panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static SEEN: RefCell<Vec<SessionChange>> = const { RefCell::new(Vec::new()) };
    }

    fn record(change: SessionChange) {
        SEEN.with(|s| s.borrow_mut().push(change));
    }

    #[test]
    fn session_notifications_map_to_changes() {
        on_session(record);
        let ws = NSWorkspace::sharedWorkspace().notificationCenter();
        // Posted by hand on this process's workspace center: nothing sleeps or switches.
        // SAFETY: immutable framework constants, set at load time.
        let names = unsafe {
            [
                NSWorkspaceWillSleepNotification,
                NSWorkspaceSessionDidResignActiveNotification,
                NSWorkspaceSessionDidBecomeActiveNotification,
            ]
        };
        for name in names {
            // SAFETY: a framework name and no object.
            unsafe { ws.postNotificationName_object(name, None) };
        }
        let seen = SEEN.with(|s| s.borrow().clone());
        assert_eq!(seen, [SessionChange::Sleep, SessionChange::Locked, SessionChange::Unlocked]);
    }
}
