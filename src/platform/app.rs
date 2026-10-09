//! The process as a macOS app: main thread, activation policy, run loop, single instance.

use std::path::PathBuf;

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

/// The `.app` bundle Flick runs from, so callers can protect it (e.g. from uninstall). `None`
/// for a bare binary such as `cargo run` or the tests.
#[cfg_attr(not(test), expect(dead_code, reason = "first caller is the app actions (flick-78f6)"))]
pub fn own_bundle() -> Option<PathBuf> {
    let path = PathBuf::from(NSBundle::mainBundle().bundlePath().to_string());
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_test_binary_has_no_own_bundle() {
        assert_eq!(own_bundle(), None);
    }
}
