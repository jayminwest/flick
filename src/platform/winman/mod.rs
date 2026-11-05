//! Window management for `tile`: every Accessibility and window-server call it needs, behind
//! plain data. Nothing runs until `start`; `stop` tears it all down.
//!
//! Each regular app gets its own named thread with its own `CFRunLoop` (`thread`), where its
//! AX observer runs and every AX call to it is made (`observe`, `frame`), so a hung app blocks
//! only its own thread. AX elements never leave their app thread: the main thread sees only
//! `WinId`s, `WinInfo`s and `Change`s. App threads keep the window cache current and queue
//! changes; the first change after a `drain` calls `on_change` (on any thread), which only
//! posts an event, never borrows app state (mx-fcbc43).
//!
//! `start`, `stop`, `drain`, `snapshot`, `refresh`, `set_frames`, `focus` and `close` run on
//! the main thread and never wait on another app.

mod corner;
mod frame;
pub mod hide;
mod ids;
mod observe;
mod thread;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

pub use frame::set_frames;
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "tile's window commands (flick-c2ea) call them")
)]
pub use frame::{close, focus};

use super::{Rect, ax, workspace};

/// A window, by its window-server id (`CGWindowID`). Ids stay valid across a Flick restart.
pub type WinId = u32;

/// The app a window belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct AppInfo {
    pub pid: i32,
    pub bundle_id: Option<String>,
    /// The app is hidden (⌘H).
    pub hidden: bool,
}

/// One window, as last seen by its app thread.
#[derive(Clone, Debug, PartialEq)]
pub struct WinInfo {
    pub id: WinId,
    pub app: AppInfo,
    /// Empty when the window has no title.
    pub title: String,
    /// In Accessibility coordinates; zero when the app did not say.
    pub frame: Rect,
    /// `AXStandardWindow`, `AXDialog`, … None when the window has none.
    pub subrole: Option<String>,
    pub minimized: bool,
    /// Native full screen (`AXFullScreen`).
    pub fullscreen: bool,
    /// The green full-screen button, as resizable document windows have.
    pub fullscreen_button: Button,
    /// The window has any title-bar button (close, minimize, zoom or full screen).
    pub title_buttons: bool,
    /// The window is its app's focused or main window.
    pub focus: Focus,
    /// The window-server level (`kCGWindowLayer`): 0 for normal windows, higher for floating
    /// panels; None when the window server did not say.
    pub level: Option<i32>,
}

/// A title-bar button of a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Missing,
    Disabled,
    Enabled,
}

/// Whether a window is its app's focused (`AXFocused`) and main (`AXMain`) window. Only one
/// window of an app is each at a time, but they can differ (a focused panel).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Focus {
    pub focused: bool,
    pub main: bool,
}

/// Something that happened since the last `drain`, oldest first. The cache behind `snapshot`
/// already holds its result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// A window appeared: it existed at `start`, its app launched, or it opened.
    Created(WinId),
    /// A window closed, or was gone when its app was rescanned.
    Destroyed(WinId),
    /// A window became its app's focused or main window; every window of that app has its
    /// `focus` read again.
    Focused(WinId),
    Minimized(WinId),
    Deminimized(WinId),
    Moved(WinId),
    Resized(WinId),
    /// A regular app launched; its windows follow as `Created`.
    Launched(i32),
    /// An app quit; its windows are gone from the cache.
    Terminated(i32),
    Hidden(i32),
    Unhidden(i32),
    /// The left mouse button went up in another app (a drag or resize may have ended). Every
    /// app is rescanned too, since `AXUIElementDestroyed` does not always come.
    MouseUp,
}

/// Apps never watched: they own no windows to manage.
const SKIP_BUNDLES: [&str; 1] = ["com.apple.loginwindow"];

/// What app threads and the main thread share: the change queue and the window cache.
pub(super) struct Shared {
    on_change: fn(),
    /// Set by `stop`: an app thread still finishing a job reports nothing more.
    stopped: AtomicBool,
    changes: Mutex<Vec<Change>>,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    windows: BTreeMap<WinId, WinInfo>,
    /// Hidden apps, kept by the main thread; `snapshot` copies it into each `AppInfo`.
    hidden: HashSet<i32>,
}

impl Shared {
    fn new(on_change: fn()) -> Shared {
        Shared {
            on_change,
            stopped: AtomicBool::new(false),
            changes: Mutex::new(Vec::new()),
            cache: Mutex::new(Cache::default()),
        }
    }

    /// Queue `change`; the first one after a `drain` calls `on_change`.
    pub(super) fn push(&self, change: Change) {
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let first = {
            let mut changes = self.changes.lock().unwrap_or_else(PoisonError::into_inner);
            changes.push(change);
            changes.len() == 1
        };
        if first {
            (self.on_change)();
        }
    }

    fn drain(&self) -> Vec<Change> {
        std::mem::take(&mut *self.changes.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn cache<R>(&self, f: impl FnOnce(&mut Cache) -> R) -> R {
        f(&mut self.cache.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Store `info`, replacing what was known about that window.
    pub(super) fn put(&self, info: WinInfo) {
        self.cache(|c| c.windows.insert(info.id, info));
    }

    pub(super) fn remove(&self, id: WinId) {
        self.cache(|c| c.windows.remove(&id));
    }

    /// Forget every window of app `pid`.
    fn remove_app(&self, pid: i32) {
        self.cache(|c| {
            c.windows.retain(|_, w| w.app.pid != pid);
            c.hidden.remove(&pid);
        });
    }

    fn set_hidden(&self, pid: i32, hidden: bool) {
        self.cache(|c| if hidden { c.hidden.insert(pid) } else { c.hidden.remove(&pid) });
    }

    fn pid_of(&self, id: WinId) -> Option<i32> {
        self.cache(|c| c.windows.get(&id).map(|w| w.app.pid))
    }

    fn snapshot(&self) -> Vec<WinInfo> {
        self.cache(|c| {
            let mut out: Vec<WinInfo> = c.windows.values().cloned().collect();
            for w in &mut out {
                w.app.hidden = c.hidden.contains(&w.app.pid);
            }
            out
        })
    }
}

/// Everything `start` installed, on the main thread.
struct Session {
    shared: Arc<Shared>,
    apps: HashMap<i32, thread::App>,
    /// Workspace observers and the mouse monitor; dropping it removes them.
    _global: observe::Global,
}

impl Session {
    /// The app thread that owns window `id`.
    fn app_of(&self, id: WinId) -> Option<&thread::App> {
        self.apps.get(&self.shared.pid_of(id)?)
    }

    /// Start an app thread for `pid`, unless it is skipped or already has one.
    fn watch(&mut self, pid: i32, hidden: bool) {
        let bundle_id = workspace::app_identity(pid).and_then(|(id, _)| id);
        if skipped(pid, bundle_id.as_deref()) || self.apps.contains_key(&pid) {
            return;
        }
        self.shared.set_hidden(pid, hidden);
        if let Some(app) = thread::App::spawn(pid, bundle_id, Arc::clone(&self.shared)) {
            self.apps.insert(pid, app);
        }
    }
}

thread_local! {
    /// The running session (main thread).
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Flick itself and `SKIP_BUNDLES` are never watched.
fn skipped(pid: i32, bundle_id: Option<&str>) -> bool {
    pid == std::process::id() as i32 || bundle_id.is_some_and(|b| SKIP_BUNDLES.contains(&b))
}

/// Watch every regular app's windows, now and as apps launch; `on_change` (any thread) hears
/// of the first change after each `drain`. False, and nothing installed, without
/// Accessibility permission. A second `start` keeps the running session.
pub fn start(on_change: fn()) -> bool {
    if running() {
        return true;
    }
    if !ax::is_trusted(false) {
        return false;
    }
    let shared = Arc::new(Shared::new(on_change));
    let global = observe::Global::install(observe::Hooks {
        launched,
        terminated,
        hidden,
        mouse_up: refresh_all,
    });
    let mut session = Session { shared, apps: HashMap::new(), _global: global };
    for app in workspace::regular_apps(std::process::id() as i32) {
        session.watch(app.pid, app.hidden);
    }
    SESSION.set(Some(session));
    true
}

/// Remove every observer and end every app thread. A thread stuck in a hung app ends when the
/// app answers; it reports nothing more. Does nothing when not started.
pub fn stop() {
    let Some(session) = SESSION.take() else { return };
    session.shared.stopped.store(true, Ordering::SeqCst);
    for app in session.apps.values() {
        app.stop();
    }
}

/// `start` ran and `stop` has not.
pub fn running() -> bool {
    SESSION.with_borrow(Option::is_some)
}

/// The changes since the last `drain`, oldest first.
pub fn drain() -> Vec<Change> {
    with_session(|s| s.shared.drain()).unwrap_or_default()
}

/// Every known window of every watched app, by id. Never asks an app: it reads the cache.
pub fn snapshot() -> Vec<WinInfo> {
    with_session(|s| s.shared.snapshot()).unwrap_or_default()
}

/// Ask every app thread to enumerate its windows again; what appeared or went missing comes
/// as `Created` and `Destroyed`.
pub fn refresh() {
    with_session(|s| s.apps.values().for_each(thread::App::rescan));
}

/// `f` on the session, unless there is none or it is borrowed (a re-entered callback).
fn with_session<R>(f: impl FnOnce(&mut Session) -> R) -> Option<R> {
    SESSION.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

fn launched(pid: i32) {
    with_session(|s| {
        if workspace::is_regular(pid) {
            s.watch(pid, false);
            s.shared.push(Change::Launched(pid));
        }
    });
}

fn terminated(pid: i32) {
    with_session(|s| {
        if let Some(app) = s.apps.remove(&pid) {
            app.stop();
            s.shared.remove_app(pid);
            s.shared.push(Change::Terminated(pid));
        }
    });
}

fn hidden(pid: i32, hidden: bool) {
    with_session(|s| {
        if s.apps.contains_key(&pid) {
            s.shared.set_hidden(pid, hidden);
            s.shared.push(if hidden { Change::Hidden(pid) } else { Change::Unhidden(pid) });
        }
    });
}

fn refresh_all() {
    with_session(|s| s.shared.push(Change::MouseUp));
    refresh();
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn count() {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }

    fn info(id: WinId, pid: i32) -> WinInfo {
        WinInfo {
            id,
            app: AppInfo { pid, bundle_id: Some("com.example.app".into()), hidden: false },
            title: format!("w{id}"),
            frame: Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            subrole: Some("AXStandardWindow".into()),
            minimized: false,
            fullscreen: false,
            fullscreen_button: Button::Enabled,
            title_buttons: true,
            focus: Focus::default(),
            level: Some(0),
        }
    }

    #[test]
    fn only_the_first_change_after_a_drain_calls_on_change() {
        let shared = Shared::new(count);
        let before = CALLS.load(Ordering::SeqCst);
        shared.push(Change::Created(1));
        shared.push(Change::Moved(1));
        assert_eq!(CALLS.load(Ordering::SeqCst), before + 1);
        assert_eq!(shared.drain(), [Change::Created(1), Change::Moved(1)]);
        assert!(shared.drain().is_empty());
        shared.push(Change::MouseUp);
        assert_eq!(CALLS.load(Ordering::SeqCst), before + 2);
        shared.stopped.store(true, Ordering::SeqCst);
        shared.push(Change::Destroyed(1));
        assert_eq!(shared.drain(), [Change::MouseUp]);
    }

    #[test]
    fn the_cache_tracks_windows_apps_and_hiding() {
        let shared = Shared::new(count);
        shared.put(info(3, 10));
        shared.put(info(1, 10));
        shared.put(info(2, 20));
        assert_eq!(shared.pid_of(2), Some(20));
        assert_eq!(shared.pid_of(9), None);
        shared.set_hidden(10, true);
        let snap = shared.snapshot();
        assert_eq!(snap.iter().map(|w| w.id).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(snap.iter().map(|w| w.app.hidden).collect::<Vec<_>>(), [true, false, true]);
        shared.set_hidden(10, false);
        assert!(shared.snapshot().iter().all(|w| !w.app.hidden));
        shared.remove(2);
        shared.remove_app(10);
        assert!(shared.snapshot().is_empty());
    }

    #[test]
    fn flick_and_loginwindow_are_skipped() {
        assert!(skipped(std::process::id() as i32, None));
        assert!(skipped(1, Some("com.apple.loginwindow")));
        assert!(!skipped(1, Some("com.apple.Safari")));
        assert!(!skipped(1, None));
    }

    #[test]
    fn nothing_runs_before_start() {
        assert!(!running());
        assert!(drain().is_empty());
        assert!(snapshot().is_empty());
        refresh();
        set_frames(&[(1, Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 })]);
        focus(1);
        close(1);
        stop();
        assert!(!running());
    }
}
