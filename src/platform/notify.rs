//! User notifications through `UNUserNotificationCenter`: ask for permission, post a banner,
//! and report clicks to a handler by the notification's id.
//!
//! `UNUserNotificationCenter` raises an Objective-C exception in a process without a bundle
//! (`cargo run`, the tests), so every call here checks for a `.app` bundle with an identifier
//! first and returns `Err` without touching the center when there is none.

use std::cell::OnceCell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, define_class, msg_send};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

/// Why a notification call did nothing.
pub const NO_BUNDLE: &str = "notifications need Flick.app (no bundle identifier)";

/// Where clicks go: set once by `on_click`, called on the main thread.
static ON_CLICK: OnceLock<fn(&str)> = OnceLock::new();

/// The answer to `request_permission`: `UNKNOWN` until the system replies.
static PERMISSION: AtomicU8 = AtomicU8::new(UNKNOWN);
const UNKNOWN: u8 = 0;
const GRANTED: u8 = 1;
const DENIED: u8 = 2;

thread_local! {
    /// The center's delegate; the center holds it weakly, so this keeps it alive.
    static DELEGATE: OnceCell<Retained<Delegate>> = const { OnceCell::new() };
}

define_class!(
    // The system may call these on a background queue, so the class is not main-thread-only;
    // the click handler itself runs on the main queue.
    #[unsafe(super(NSObject))]
    #[name = "FlickNotificationDelegate"]
    struct Delegate;

    // SAFETY: the protocols' methods are optional; the ones below have their exact signatures.
    unsafe impl NSObjectProtocol for Delegate {}
    // SAFETY: as above.
    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        // Flick is often frontmost (the launcher is open): show the banner anyway.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            let id = response.notification().request().identifier().to_string();
            super::events::on_main(move || clicked(&id));
            completion.call(());
        }
    }
);

/// Whether Flick runs from a `.app` bundle with an identifier, which notifications need.
pub fn available() -> bool {
    let bundle = NSBundle::mainBundle();
    let is_app = bundle.bundlePath().to_string().to_ascii_lowercase().ends_with(".app");
    is_app && bundle.bundleIdentifier().is_some_and(|id| !id.is_empty())
}

/// The notification center, with Flick's delegate installed, or `Err` without a bundle.
fn center() -> Result<Retained<UNUserNotificationCenter>, &'static str> {
    if !available() {
        return Err(NO_BUNDLE);
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    DELEGATE.with(|cell| {
        let delegate = cell.get_or_init(|| {
            // SAFETY: plain `init` on a freshly allocated NSObject subclass with no ivars.
            unsafe { msg_send![Delegate::alloc(), init] }
        });
        center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
    });
    Ok(center)
}

/// Ask once for permission to show banners with sound. macOS prompts only the first time;
/// later calls report the saved answer. `permitted` holds the answer once the system replies.
pub fn request_permission() -> Result<(), &'static str> {
    let center = center()?;
    let done = RcBlock::new(|granted: Bool, _error: *mut NSError| {
        let answer = if granted.as_bool() { GRANTED } else { DENIED };
        PERMISSION.store(answer, Ordering::Relaxed);
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &done,
    );
    Ok(())
}

/// The answer to the last `request_permission`: `None` before the system replies.
pub fn permitted() -> Option<bool> {
    match PERMISSION.load(Ordering::Relaxed) {
        GRANTED => Some(true),
        DENIED => Some(false),
        _ => None,
    }
}

/// Show a notification now. `id` names it: a second post with the same id replaces the first,
/// and a click on it calls the `on_click` handler with `id`. Delivery fails silently when the
/// user has not allowed notifications.
pub fn post(id: &str, title: &str, body: &str) -> Result<(), &'static str> {
    let center = center()?;
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(id),
        &content,
        None,
    );
    center.addNotificationRequest_withCompletionHandler(&request, None);
    Ok(())
}

/// Call `handler` with a notification's id when the user clicks it, on the main thread. Set
/// once; later calls are ignored. Set it at launch: a click that starts Flick arrives early.
pub fn on_click(handler: fn(&str)) -> Result<(), &'static str> {
    let _ = ON_CLICK.set(handler);
    center().map(drop)
}

/// Main thread: hand a clicked notification's id to the handler, if one is set.
fn clicked(id: &str) {
    if let Some(handler) = ON_CLICK.get() {
        handler(id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static CLICKS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn record(id: &str) {
        CLICKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(id.to_owned());
    }

    // The test binary has no bundle, so every call takes the no-op path and never reaches
    // the notification center (which would raise).
    #[test]
    fn unbundled_process_is_a_no_op() {
        assert!(!available());
        assert_eq!(request_permission(), Err(NO_BUNDLE));
        assert_eq!(post("m/p1", "agent is waiting", "m · ~/src"), Err(NO_BUNDLE));
        assert_eq!(permitted(), None);
        assert!(DELEGATE.with(|cell| cell.get().is_none()));
    }

    #[test]
    fn click_goes_to_the_handler_set_first() {
        assert_eq!(on_click(record), Err(NO_BUNDLE));
        assert_eq!(on_click(|_| {}), Err(NO_BUNDLE));
        clicked("mbp-server/p7");
        let clicks = CLICKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(*clicks, ["mbp-server/p7"]);
    }
}
