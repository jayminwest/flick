//! Running apps, opening files and URLs, and app activation (`NSWorkspace`).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSString, NSURL};

thread_local! {
    static OBSERVER: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> = const { RefCell::new(None) };
}

/// Open `url` in its default app.
pub fn open_url(url: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

/// Open the file or app bundle at `path`.
pub fn open_file(path: &Path) {
    NSWorkspace::sharedWorkspace()
        .openURL(&NSURL::fileURLWithPath(&NSString::from_str(&path.display().to_string())));
}

pub fn frontmost_pid() -> Option<i32> {
    NSWorkspace::sharedWorkspace().frontmostApplication().map(|a| a.processIdentifier())
}

/// Call `f` after each app activation. Replaces the previous observer.
pub fn on_app_activated(f: impl Fn() + 'static) {
    let block = RcBlock::new(move |_n: NonNull<NSNotification>| f());
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: the notification name is an immutable framework constant, and the block takes
    // the `NSNotification` argument the center passes.
    let name = unsafe { NSWorkspaceDidActivateApplicationNotification };
    // SAFETY: the block is 'static and only runs on the posting (main) thread; the returned
    // observer is kept alive below, which keeps the registration.
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    OBSERVER.with(|o| *o.borrow_mut() = Some(observer));
}

/// App `pid` is a regular (Dock) app, not a menu-bar or background one.
pub fn is_regular(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .is_some_and(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular)
}

/// Pids of running apps that are not hidden, in the system's order.
pub fn unhidden_app_pids() -> Vec<i32> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|a| !a.isHidden())
        .map(|a| a.processIdentifier())
        .collect()
}

/// A running regular (Dock) app.
pub struct RunningApp {
    pub pid: i32,
    pub name: String,
    pub bundle: Option<PathBuf>,
    pub hidden: bool,
}

/// Running regular apps other than `except`, in the system's order.
pub fn regular_apps(except: i32) -> Vec<RunningApp> {
    let mut out = Vec::new();
    for app in &NSWorkspace::sharedWorkspace().runningApplications() {
        let pid = app.processIdentifier();
        if pid == except || app.activationPolicy() != NSApplicationActivationPolicy::Regular {
            continue;
        }
        out.push(RunningApp {
            pid,
            name: app.localizedName().map(|n| n.to_string()).unwrap_or_default(),
            bundle: app.bundleURL().and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string())),
            hidden: app.isHidden(),
        });
    }
    out
}

/// Activate app `pid` the way a Dock click does, which also switches to its Space.
/// Needs no Accessibility permission. `Err` when no such app runs, `Ok(false)` when it has no
/// bundle to open.
pub fn open_app(pid: i32) -> Result<bool, &'static str> {
    let app =
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid).ok_or("App quit")?;
    Ok(open_running(&app))
}

pub(super) fn open_running(app: &NSRunningApplication) -> bool {
    let Some(url) = app.bundleURL() else { return false };
    NSWorkspace::sharedWorkspace().openURL(&url)
}

/// Hide the frontmost app, like cmd+H.
pub fn hide_frontmost() -> Result<(), &'static str> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication().ok_or("No frontmost app")?;
    app.hide();
    Ok(())
}
