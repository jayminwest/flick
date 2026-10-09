//! The process as a macOS app: main thread, activation policy, run loop, single instance.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::Once;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationWillTerminateNotification,
    NSRunningApplication,
};
use objc2_foundation::{NSBundle, NSNotification, NSNotificationCenter, NSObjectProtocol};

#[repr(C)]
struct SourceType {
    _private: [u8; 0],
}

unsafe extern "C" {
    /// `DISPATCH_SOURCE_TYPE_SIGNAL` is the address of this symbol.
    static _dispatch_source_type_signal: SourceType;
    fn dispatch_source_create(
        kind: *const SourceType,
        handle: usize,
        mask: usize,
        queue: *const c_void,
    ) -> *mut c_void;
    fn dispatch_source_set_event_handler_f(
        source: *mut c_void,
        handler: extern "C" fn(*mut c_void),
    );
    fn dispatch_resume(object: *mut c_void);
    fn signal(sig: i32, handler: usize) -> usize;
}

const SIGTERM: i32 = 15;
const SIG_IGN: usize = 1;

thread_local! {
    /// `on_terminate` observers; dropping one ends its registration.
    static TERMINATE: RefCell<Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>> =
        const { RefCell::new(Vec::new()) };
}

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

/// Run `hook` on the main thread when the app quits through `quit` (or Quit from any menu):
/// `applicationWillTerminate`, just before exit. Not on a crash or a signal (launchd stop).
pub fn on_terminate(hook: impl Fn() + 'static) {
    let block = RcBlock::new(move |_n: NonNull<NSNotification>| hook());
    // SAFETY: the name is an immutable framework constant; the block is 'static, takes the
    // `NSNotification` argument the center passes, and runs on the posting (main) thread; the
    // observer is kept below, which keeps the registration.
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationWillTerminateNotification),
            None,
            None,
            &block,
        )
    };
    TERMINATE.with(|t| t.borrow_mut().push(observer));
}

/// Quit through `quit` on SIGTERM (launchd stop, `kill`, scripts/relaunch.sh), so the
/// `on_terminate` hooks run then too. Installs once; later calls do nothing.
pub fn quit_on_sigterm() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: a valid source type, signal number and mask; the main queue is static.
        let source = unsafe {
            dispatch_source_create(
                &raw const _dispatch_source_type_signal,
                SIGTERM as usize,
                0,
                super::events::main_queue(),
            )
        };
        if source.is_null() {
            eprintln!("flick: no SIGTERM handler; quit by signal skips cleanup");
            return;
        }
        // SAFETY: `source` is live and not yet resumed; the handler is a plain function that
        // ignores its (null) context. The source is never released: it lives as long as Flick.
        unsafe { dispatch_source_set_event_handler_f(source, on_sigterm) };
        // SAFETY: as above; a new source starts suspended and is resumed once.
        unsafe { dispatch_resume(source) };
        // SAFETY: SIG_IGN is a valid disposition. Without it TERM kills Flick before the
        // source sees it; the source still receives an ignored signal.
        unsafe { signal(SIGTERM, SIG_IGN) };
    });
}

/// Main queue, on SIGTERM.
extern "C" fn on_sigterm(_context: *mut c_void) {
    quit();
    // `terminate:` exits; if it did not, exit as TERM asks.
    std::process::exit(0);
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
    fn on_terminate_runs_the_hook_on_will_terminate() {
        use std::cell::Cell;
        use std::rc::Rc;

        let ran = Rc::new(Cell::new(0));
        let seen = Rc::clone(&ran);
        on_terminate(move || seen.set(seen.get() + 1));
        // Posted by hand, without an NSApplication: nothing quits.
        // SAFETY: an immutable framework constant as the name, and no object.
        unsafe {
            NSNotificationCenter::defaultCenter()
                .postNotificationName_object(NSApplicationWillTerminateNotification, None);
        }
        assert_eq!(ran.get(), 1);
    }

    #[test]
    fn a_bare_test_binary_has_no_own_bundle() {
        assert_eq!(own_bundle(), None);
    }
}
