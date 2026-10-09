//! The process as a macOS app: main thread, activation policy, run loop, single instance.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSRunningApplication};
use objc2_foundation::NSBundle;

/// Panic unless called on the main thread; `AppKit` works nowhere else.
#[expect(clippy::expect_used, reason = "startup invariant: no UI without the main thread")]
pub fn require_main_thread() {
    MainThreadMarker::new().expect("must start on the main thread");
}

/// Run as an accessory app: no Dock icon or menu bar.
pub fn set_accessory() {
    NSApplication::sharedApplication(super::mtm())
        .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

/// Run the main run loop until the app quits.
pub fn run() {
    NSApplication::sharedApplication(super::mtm()).run();
}

pub fn quit() {
    NSApplication::sharedApplication(super::mtm()).terminate(None);
}

/// Another Flick.app is running (e.g. opened by hand next to the login agent's copy).
pub fn already_running() -> bool {
    let Some(id) = NSBundle::mainBundle().bundleIdentifier() else { return false };
    // Compare pids: a process launchd starts directly may not be registered yet itself.
    let me = std::process::id() as i32;
    NSRunningApplication::runningApplicationsWithBundleIdentifier(&id)
        .iter()
        .any(|app| app.processIdentifier() != me)
}
