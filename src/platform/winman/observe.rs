//! What tells winman that windows changed.
//!
//! On each app thread: one `AXObserver` with the app notifications `AXWindowCreated`,
//! `AXFocusedWindowChanged` and `AXMainWindowChanged`, and per window `AXUIElementDestroyed`,
//! `AXWindowMiniaturized`, `AXWindowDeminiaturized`, `AXMoved` and `AXResized` (`AeroSpace`
//! `MacApp.swift:70-72,435-439`). A window's notifications carry its id as their refcon.
//! The windows that exist when the thread starts come from one enumeration of `AXWindows`.
//!
//! On the main thread (`Global`): `NSWorkspace` launch, terminate, hide and unhide, and a
//! global left-mouse-up monitor, since `AXUIElementDestroyed` is unreliable (`AeroSpace`
//! `GlobalObserver.swift:47-77`): a mouse up rescans every app.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::panic::catch_unwind;
use std::ptr::{self, NonNull};
use std::sync::Arc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::{
    NSEvent, NSEventMask, NSWorkspace, NSWorkspaceDidHideApplicationNotification,
    NSWorkspaceDidLaunchApplicationNotification, NSWorkspaceDidTerminateApplicationNotification,
    NSWorkspaceDidUnhideApplicationNotification,
};
use objc2_foundation::{
    NSArray, NSNotification, NSNotificationCenter, NSNotificationName, NSObjectProtocol, NSString,
};

use super::thread::Job;
use super::{AppInfo, Button, Change, Focus, Shared, WinId, WinInfo, ids};
use crate::platform::ax::{self, CFTypeRef, Cf};
use crate::platform::{Rect, events};

/// App notifications; their element is a window.
const CREATED: &str = "AXWindowCreated";
const FOCUSED: &str = "AXFocusedWindowChanged";
const MAIN: &str = "AXMainWindowChanged";
/// Window notifications.
const DESTROYED: &str = "AXUIElementDestroyed";
const MINIATURIZED: &str = "AXWindowMiniaturized";
const DEMINIATURIZED: &str = "AXWindowDeminiaturized";
const MOVED: &str = "AXMoved";
const RESIZED: &str = "AXResized";
const APP_NOTES: [&str; 3] = [CREATED, FOCUSED, MAIN];
const WINDOW_NOTES: [&str; 5] = [DESTROYED, MINIATURIZED, DEMINIATURIZED, MOVED, RESIZED];

/// Title-bar buttons of a window, by attribute.
const FULLSCREEN_BUTTON: &str = "AXFullScreenButton";
const TITLE_BUTTONS: [&str; 4] =
    ["AXCloseButton", "AXMinimizeButton", "AXZoomButton", FULLSCREEN_BUTTON];

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
    static kCFRunLoopDefaultMode: CFTypeRef;
    fn CFRunLoopGetCurrent() -> CFTypeRef;
    fn CFRunLoopAddSource(rl: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
    fn CFRunLoopRemoveSource(rl: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
}

/// What one AX notification means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Created,
    Focused,
    Destroyed,
    Minimized,
    Deminimized,
    Moved,
    Resized,
}

fn kind(name: &str) -> Option<Kind> {
    Some(match name {
        CREATED => Kind::Created,
        FOCUSED | MAIN => Kind::Focused,
        DESTROYED => Kind::Destroyed,
        MINIATURIZED => Kind::Minimized,
        DEMINIATURIZED => Kind::Deminimized,
        MOVED => Kind::Moved,
        RESIZED => Kind::Resized,
        _ => return None,
    })
}

/// One notification, queued until the app's state is free.
struct Note {
    kind: Kind,
    /// The window whose notification it is (its refcon); None for app notifications.
    window: Option<WinId>,
    element: Retained<AnyObject>,
}

thread_local! {
    /// This app thread's observed app (app threads only).
    static WATCHED: RefCell<Option<Watched>> = const { RefCell::new(None) };
    /// Notifications not yet handled.
    static NOTES: RefCell<VecDeque<Note>> = const { RefCell::new(VecDeque::new()) };
}

/// One app as its thread sees it: its AX element, observer and windows.
pub(super) struct Watched {
    pid: i32,
    bundle_id: Option<String>,
    shared: Arc<Shared>,
    app: Cf,
    observer: Cf,
    windows: HashMap<WinId, Retained<AnyObject>>,
}

impl Watched {
    /// Observe app `pid` on this thread's run loop and register the windows it has. None
    /// when the app does not answer AX (not yet launched, quit, or no permission).
    pub(super) fn new(pid: i32, bundle_id: Option<String>, shared: Arc<Shared>) -> Option<Self> {
        let app = ax::app_element(pid);
        if app.0.is_null() {
            return None;
        }
        let mut observer: CFTypeRef = ptr::null();
        // SAFETY: `on_note` matches `AXObserverCallback`; `observer` receives a +1 reference
        // on success, which `Cf` releases.
        if unsafe { AXObserverCreate(pid, on_note, &mut observer) } != 0 || observer.is_null() {
            return None;
        }
        let observer = Cf(observer);
        // All or nothing: an app that is still launching refuses them, and is tried again.
        if !APP_NOTES.iter().all(|n| add(&observer, app.0, n, None)) {
            return None;
        }
        // SAFETY: the observer is live; its source is a Get-rule reference it owns.
        let source = unsafe { AXObserverGetRunLoopSource(observer.0) };
        // SAFETY: plain C call; the current thread's run loop, not retained.
        let rl = unsafe { CFRunLoopGetCurrent() };
        // SAFETY: this thread's run loop, a live source and a framework mode constant.
        unsafe { CFRunLoopAddSource(rl, source, kCFRunLoopDefaultMode) };
        let mut watched =
            Watched { pid, bundle_id, shared, app, observer, windows: HashMap::new() };
        watched.rescan();
        Some(watched)
    }

    /// The app's AX element.
    pub(super) fn app(&self) -> CFTypeRef {
        self.app.0
    }

    /// Window `id`'s AX element, if the app has it.
    pub(super) fn window(&self, id: WinId) -> Option<CFTypeRef> {
        self.windows.get(&id).map(|w| Retained::as_ptr(w).cast())
    }

    /// Enumerate `AXWindows`: register new windows, drop the ones that are gone.
    pub(super) fn rescan(&mut self) {
        let windows =
            ax::copy_attr(self.app.0, "AXWindows").and_then(|w| w.downcast::<NSArray>().ok());
        let mut seen = HashSet::new();
        for win in windows.iter().flat_map(|w| w.iter()) {
            seen.extend(self.register(win));
        }
        for id in missing(self.windows.keys().copied(), &seen) {
            self.unregister(id);
        }
    }

    /// Know window `el` (no-op for a known one) and return its id; None if it is no window.
    fn register(&mut self, el: Retained<AnyObject>) -> Option<WinId> {
        let ptr: CFTypeRef = Retained::as_ptr(&el).cast();
        let id = ids::window_id(ptr)?;
        if self.windows.contains_key(&id) {
            return Some(id);
        }
        for name in WINDOW_NOTES {
            add(&self.observer, ptr, name, Some(id));
        }
        self.windows.insert(id, el);
        self.update(id);
        self.shared.push(Change::Created(id));
        Some(id)
    }

    fn unregister(&mut self, id: WinId) {
        let Some(el) = self.windows.remove(&id) else { return };
        for name in WINDOW_NOTES {
            remove(&self.observer, Retained::as_ptr(&el).cast(), name);
        }
        self.shared.remove(id);
        self.shared.push(Change::Destroyed(id));
    }

    /// Read window `id` again into the shared cache.
    fn update(&self, id: WinId) {
        let Some(el) = self.window(id) else { return };
        self.shared.put(WinInfo {
            id,
            app: AppInfo { pid: self.pid, bundle_id: self.bundle_id.clone(), hidden: false },
            title: ax::string_attr(el, "AXTitle").unwrap_or_default(),
            frame: ax::frame(el).unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }),
            subrole: ax::string_attr(el, "AXSubrole"),
            minimized: ax::bool_attr(el, "AXMinimized"),
            fullscreen: ax::bool_attr(el, "AXFullScreen"),
            fullscreen_button: button(el, FULLSCREEN_BUTTON),
            title_buttons: TITLE_BUTTONS.iter().any(|b| button(el, b) != Button::Missing),
            focus: Focus {
                focused: ax::bool_attr(el, "AXFocused"),
                main: ax::bool_attr(el, "AXMain"),
            },
            level: ids::level(id),
        });
    }

    fn handle(&mut self, note: Note) {
        let (id, change): (WinId, fn(WinId) -> Change) = match (note.kind, note.window) {
            (Kind::Created, _) => {
                self.register(note.element);
                return;
            }
            (Kind::Focused, _) => {
                let Some(id) = self.register(note.element) else { return };
                // Focus left another window of this app: read every window's `focus` again.
                let mut ids: Vec<_> = self.windows.keys().copied().filter(|w| *w != id).collect();
                ids.sort_unstable();
                for w in ids {
                    self.update(w);
                }
                (id, Change::Focused)
            }
            (Kind::Destroyed, Some(id)) => {
                self.unregister(id);
                return;
            }
            (Kind::Minimized, Some(id)) => (id, Change::Minimized),
            (Kind::Deminimized, Some(id)) => (id, Change::Deminimized),
            (Kind::Moved, Some(id)) => (id, Change::Moved),
            (Kind::Resized, Some(id)) => (id, Change::Resized),
            (_, None) => return,
        };
        if self.windows.contains_key(&id) {
            self.update(id);
            self.shared.push(change(id));
        }
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        // SAFETY: the observer is live (released after this, by its `Cf`).
        let source = unsafe { AXObserverGetRunLoopSource(self.observer.0) };
        // SAFETY: plain C call; the current thread's run loop, not retained.
        let rl = unsafe { CFRunLoopGetCurrent() };
        // SAFETY: `new` added the source to this thread's run loop, and `Watched` never
        // leaves the thread.
        unsafe { CFRunLoopRemoveSource(rl, source, kCFRunLoopDefaultMode) };
    }
}

/// Window `el`'s title-bar button `attr`, and whether it is enabled.
fn button(el: CFTypeRef, attr: &str) -> Button {
    match ax::copy_attr(el, attr) {
        None => Button::Missing,
        Some(b) if ax::bool_attr(Retained::as_ptr(&b).cast(), "AXEnabled") => Button::Enabled,
        Some(_) => Button::Disabled,
    }
}

/// The ids in `known` that a rescan did not see.
fn missing(known: impl Iterator<Item = WinId>, seen: &HashSet<WinId>) -> Vec<WinId> {
    let mut out: Vec<_> = known.filter(|id| !seen.contains(id)).collect();
    out.sort_unstable();
    out
}

/// Make `watched` this thread's app.
pub(super) fn install(watched: Watched) {
    WATCHED.set(Some(watched));
}

/// Forget this thread's app: its observer leaves the run loop.
pub(super) fn uninstall() {
    NOTES.with_borrow_mut(VecDeque::clear);
    drop(WATCHED.take());
}

/// Run `job` on this thread's app, then handle the notifications that came meanwhile.
pub(super) fn with(job: Job) {
    WATCHED.with(|w| {
        if let Ok(mut w) = w.try_borrow_mut()
            && let Some(w) = w.as_mut()
        {
            job(w);
        }
    });
    flush();
}

/// Handle queued notifications, unless the app's state is borrowed (then its holder flushes).
fn flush() {
    WATCHED.with(|w| {
        let Ok(mut w) = w.try_borrow_mut() else { return };
        let Some(w) = w.as_mut() else { return };
        while let Some(note) = NOTES.with_borrow_mut(VecDeque::pop_front) {
            w.handle(note);
        }
    });
}

fn add(observer: &Cf, el: CFTypeRef, name: &str, window: Option<WinId>) -> bool {
    let name = NSString::from_str(name);
    let refcon = window.map_or(ptr::null_mut(), |id| id as usize as *mut c_void);
    // SAFETY: live observer, element and name; the refcon is a plain number, never followed.
    let err = unsafe { AXObserverAddNotification(observer.0, el, ax::cfstr(&name), refcon) };
    // kAXErrorNotificationAlreadyRegistered.
    err == 0 || err == -25209
}

fn remove(observer: &Cf, el: CFTypeRef, name: &str) {
    let name = NSString::from_str(name);
    // SAFETY: live observer, element and name; an element that is gone just fails.
    unsafe { AXObserverRemoveNotification(observer.0, el, ax::cfstr(&name)) };
}

/// App thread, inside its run loop: queue one AX notification, then handle the queue.
unsafe extern "C" fn on_note(_obs: CFTypeRef, el: CFTypeRef, name: CFTypeRef, refcon: *mut c_void) {
    // SAFETY: AX passes a live CFString (toll-free bridged to NSString) for the call, or null.
    let name = unsafe { name.cast::<NSString>().as_ref() }.map(NSString::to_string);
    let Some(kind) = name.as_deref().and_then(kind) else { return };
    // SAFETY: AX passes a live element for the call; retaining keeps it past the call.
    let Some(element) = (unsafe { Retained::retain(el as *mut AnyObject) }) else { return };
    let window = (refcon as usize != 0).then_some(refcon as usize as WinId);
    NOTES.with_borrow_mut(|n| n.push_back(Note { kind, window, element }));
    if catch_unwind(flush).is_err() {
        eprintln!("flick: winman notification panicked");
    }
}

/// Main-thread handlers for `Global`; each only borrows winman's own session.
#[derive(Clone, Copy)]
pub(super) struct Hooks {
    pub(super) launched: fn(i32),
    pub(super) terminated: fn(i32),
    pub(super) hidden: fn(i32, bool),
    pub(super) mouse_up: fn(),
}

/// The main-thread observers; dropping it removes them.
pub(super) struct Global {
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    monitor: Option<Retained<AnyObject>>,
}

impl Global {
    pub(super) fn install(hooks: Hooks) -> Global {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: notification names are immutable framework constants, set at load time.
        let (launch, terminate, hide, unhide) = unsafe {
            (
                NSWorkspaceDidLaunchApplicationNotification,
                NSWorkspaceDidTerminateApplicationNotification,
                NSWorkspaceDidHideApplicationNotification,
                NSWorkspaceDidUnhideApplicationNotification,
            )
        };
        let Hooks { launched, terminated, hidden, mouse_up } = hooks;
        let observers = vec![
            observe(&center, launch, launched),
            observe(&center, terminate, terminated),
            observe(&center, hide, move |pid| hidden(pid, true)),
            observe(&center, unhide, move |pid| hidden(pid, false)),
        ];
        let block = RcBlock::new(move |_: NonNull<NSEvent>| mouse_up());
        let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseUp,
            &block,
        );
        Global { observers, monitor }
    }
}

impl Drop for Global {
    fn drop(&mut self) {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        for observer in &self.observers {
            // SAFETY: each came from this center's addObserverForName and is removed once.
            unsafe { center.removeObserver(observer.as_ref()) };
        }
        if let Some(monitor) = self.monitor.take() {
            // SAFETY: it came from addGlobalMonitorForEvents and is removed once (taken).
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    }
}

/// Call `f(pid)` on each `name` notification from `center` about an app.
fn observe(
    center: &NSNotificationCenter,
    name: &NSNotificationName,
    f: impl Fn(i32) + 'static,
) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let block = RcBlock::new(move |n: NonNull<NSNotification>| {
        // SAFETY: the center passes a valid notification that lives for the call.
        if let Some(pid) = events::app_pid(unsafe { n.as_ref() }) {
            f(pid);
        }
    });
    // SAFETY: the block is 'static, takes the `NSNotification` the center passes, and runs on
    // the posting (main) thread; `Global` keeps the observer until it removes it.
    unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_names_map_to_kinds() {
        assert_eq!(kind(CREATED), Some(Kind::Created));
        assert_eq!(kind(FOCUSED), Some(Kind::Focused));
        assert_eq!(kind(MAIN), Some(Kind::Focused));
        assert_eq!(kind(DESTROYED), Some(Kind::Destroyed));
        assert_eq!(kind(MINIATURIZED), Some(Kind::Minimized));
        assert_eq!(kind(DEMINIATURIZED), Some(Kind::Deminimized));
        assert_eq!(kind(MOVED), Some(Kind::Moved));
        assert_eq!(kind(RESIZED), Some(Kind::Resized));
        assert_eq!(kind("AXTitleChanged"), None);
    }

    #[test]
    fn a_rescan_drops_only_unseen_windows() {
        let seen = HashSet::from([2, 4]);
        assert_eq!(missing([5, 4, 1, 2].into_iter(), &seen), [1, 5]);
        assert!(missing([2].into_iter(), &seen).is_empty());
    }

    #[test]
    fn a_pid_that_is_no_app_is_not_watched() {
        assert!(Watched::new(-1, None, Arc::new(Shared::new(|| {}))).is_none());
    }
}
