//! Follow one app's focused window through an Accessibility observer: focus moving between
//! its windows, and title changes of the focused window. Nothing is installed until `follow`,
//! and `stop` removes it, so a caller that never reads titles never holds an observer.
//!
//! Changes are coalesced on the trailing edge: the first change arms a one-shot
//! `COALESCE_SECS` timer and later changes fold into it, so a title that changes many times a
//! second (a terminal spinner) costs at most one callback per second. Nothing polls.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::NSString;

use super::ax::{self, CFTypeRef, Cf};
use super::timer;

/// Seconds from the first change of a burst to the callback.
const COALESCE_SECS: f64 = 1.0;
const FOCUS_CHANGED: &str = "AXFocusedWindowChanged";
const TITLE_CHANGED: &str = "AXTitleChanged";
const DESTROYED: &str = "AXUIElementDestroyed";

/// `AXObserverCallback`: observer, element, notification name, refcon.
type Callback = unsafe extern "C" fn(CFTypeRef, CFTypeRef, CFTypeRef, *mut c_void);

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXObserverCreate(pid: i32, callback: Callback, out: *mut CFTypeRef) -> i32;
    fn AXObserverAddNotification(
        observer: CFTypeRef,
        el: CFTypeRef,
        name: CFTypeRef,
        refcon: *mut c_void,
    ) -> i32;
    fn AXObserverRemoveNotification(observer: CFTypeRef, el: CFTypeRef, name: CFTypeRef) -> i32;
    fn AXObserverGetRunLoopSource(observer: CFTypeRef) -> CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: CFTypeRef;
    fn CFRunLoopGetMain() -> CFTypeRef;
    fn CFRunLoopAddSource(rl: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
    fn CFRunLoopRemoveSource(rl: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
}

/// The followed pid and its callback.
type Target = (i32, fn(i32));

thread_local! {
    /// The one installed observer (main thread).
    static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
    /// Who a coalesced change goes to: the followed pid and its callback.
    static TARGET: Cell<Option<Target>> = const { Cell::new(None) };
    /// A change is waiting for the coalescing timer.
    static PENDING: Cell<bool> = const { Cell::new(false) };
    /// Bumped by `stop`, so a timer armed for an earlier target does nothing.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// Call `on_change(pid)` (on the main thread, coalesced) when app `pid`'s focused window or
/// its title changes. Replaces any earlier `follow`; following the same pid again only swaps
/// the callback. Returns false, and follows nothing, without Accessibility permission or for a
/// pid that is not a running app.
pub fn follow(pid: i32, on_change: fn(i32)) -> bool {
    if following() == Some(pid) {
        TARGET.set(Some((pid, on_change)));
        return true;
    }
    stop();
    let Some(watch) = Watch::new(pid) else {
        return false;
    };
    WATCH.with(|w| *w.borrow_mut() = Some(watch));
    TARGET.set(Some((pid, on_change)));
    true
}

/// Remove the observer, if any, and drop a pending change.
pub fn stop() {
    GENERATION.set(GENERATION.get().wrapping_add(1));
    PENDING.set(false);
    TARGET.set(None);
    let old = WATCH.with(|w| w.borrow_mut().take());
    drop(old);
}

/// The pid being followed, if an observer is installed.
pub fn following() -> Option<i32> {
    WATCH.with(|w| w.borrow().as_ref().map(|w| w.pid))
}

/// One `AXObserver` on an app, scheduled on the main run loop while it lives.
struct Watch {
    pid: i32,
    observer: Cf,
    app: Cf,
    /// The focused window, observed for title changes and destruction.
    window: Option<Retained<AnyObject>>,
}

impl Watch {
    fn new(pid: i32) -> Option<Watch> {
        let app = ax::app_element(pid);
        if app.0.is_null() {
            return None;
        }
        let mut observer: CFTypeRef = ptr::null();
        // SAFETY: `on_notification` matches `AXObserverCallback`; `observer` receives a +1
        // reference on success, which `Cf` releases.
        if unsafe { AXObserverCreate(pid, on_notification, &mut observer) } != 0 {
            return None;
        }
        let observer = Cf(observer);
        // Fails without Accessibility permission: then nothing reaches the run loop.
        if observer.0.is_null() || !add(&observer, app.0, FOCUS_CHANGED) {
            return None;
        }
        // SAFETY: the observer is live; its run loop source is a Get-rule reference it owns.
        let source = unsafe { AXObserverGetRunLoopSource(observer.0) };
        let (rl, modes) = main_loop();
        // SAFETY: the main run loop, a live source and the common-modes constant.
        unsafe { CFRunLoopAddSource(rl, source, modes) };
        let mut watch = Watch { pid, observer, app, window: None };
        watch.refocus();
        Some(watch)
    }

    /// Move the title and destruction notifications to the app's current focused window.
    fn refocus(&mut self) {
        if let Some(old) = self.window.take() {
            let el = Retained::as_ptr(&old) as CFTypeRef;
            remove(&self.observer, el, TITLE_CHANGED);
            remove(&self.observer, el, DESTROYED);
        }
        self.window = ax::copy_attr(self.app.0, "AXFocusedWindow");
        if let Some(win) = &self.window {
            let el = Retained::as_ptr(win) as CFTypeRef;
            add(&self.observer, el, TITLE_CHANGED);
            add(&self.observer, el, DESTROYED);
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: the observer is live (released after this, by its `Cf`); its source was
        // added to the main run loop in `new`.
        let source = unsafe { AXObserverGetRunLoopSource(self.observer.0) };
        let (rl, modes) = main_loop();
        // SAFETY: as in `new`; removing the source stops every callback from this observer.
        unsafe { CFRunLoopRemoveSource(rl, source, modes) };
    }
}

/// The main run loop and `kCFRunLoopCommonModes`, so changes also arrive while a menu tracks.
fn main_loop() -> (CFTypeRef, CFTypeRef) {
    // SAFETY: plain C call; the main run loop lives as long as the process (Get rule).
    let rl = unsafe { CFRunLoopGetMain() };
    // SAFETY: an immutable framework constant, set at load time.
    (rl, unsafe { kCFRunLoopCommonModes })
}

fn add(observer: &Cf, el: CFTypeRef, name: &str) -> bool {
    let name = NSString::from_str(name);
    // SAFETY: live observer, element and name; the callback ignores the null refcon.
    unsafe { AXObserverAddNotification(observer.0, el, ax::cfstr(&name), ptr::null_mut()) == 0 }
}

fn remove(observer: &Cf, el: CFTypeRef, name: &str) {
    let name = NSString::from_str(name);
    // SAFETY: live observer, element and name; an element that is gone just fails.
    unsafe { AXObserverRemoveNotification(observer.0, el, ax::cfstr(&name)) };
}

/// Main run loop: one AX notification. Focus moves and destroyed windows re-target the title
/// notification; every notification counts as a change.
unsafe extern "C" fn on_notification(
    _observer: CFTypeRef,
    _el: CFTypeRef,
    name: CFTypeRef,
    _refcon: *mut c_void,
) {
    // SAFETY: AX passes a live CFString (toll-free bridged to NSString) for the call, or null.
    let title =
        unsafe { name.cast::<NSString>().as_ref() }.is_some_and(|n| n.to_string() == TITLE_CHANGED);
    if !title {
        // `try_borrow_mut`: never re-enter a borrow held by `follow` or `stop`.
        WATCH.with(|w| {
            if let Some(watch) = w.try_borrow_mut().ok().as_deref_mut().and_then(Option::as_mut) {
                watch.refocus();
            }
        });
    }
    changed();
}

/// Note a change: arm the coalescing timer unless one is already waiting.
fn changed() {
    if TARGET.get().is_none() || PENDING.replace(true) {
        return;
    }
    let generation = GENERATION.get();
    timer::after(COALESCE_SECS, move || fire(generation));
}

/// The coalescing timer: report the waiting change, unless `stop` ran since it was armed.
fn fire(generation: u64) {
    if GENERATION.get() != generation || !PENDING.replace(false) {
        return;
    }
    if let Some((pid, on_change)) = TARGET.get() {
        on_change(pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static SEEN: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    }

    fn record(pid: i32) {
        SEEN.with(|s| s.borrow_mut().push(pid));
    }

    fn seen() -> Vec<i32> {
        SEEN.with(|s| s.borrow().clone())
    }

    #[test]
    fn a_pid_that_is_no_app_installs_nothing() {
        assert!(!follow(-1, record));
        assert_eq!(following(), None);
        stop();
        assert_eq!(following(), None);
    }

    #[test]
    fn a_burst_of_changes_reports_once() {
        // The timer is armed on this test thread's run loop, which never runs: `fire` is
        // called by hand.
        TARGET.set(Some((7, record)));
        changed();
        changed();
        changed();
        let generation = GENERATION.get();
        fire(generation);
        fire(generation);
        assert_eq!(seen(), [7]);

        changed();
        fire(generation);
        assert_eq!(seen(), [7, 7]);
    }

    #[test]
    fn stop_drops_a_pending_change() {
        TARGET.set(Some((9, record)));
        changed();
        let generation = GENERATION.get();
        stop();
        fire(generation);
        assert_eq!(seen(), Vec::<i32>::new());
        // Without a target, a change arms nothing.
        changed();
        assert!(!PENDING.get());
    }
}
