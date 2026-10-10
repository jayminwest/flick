//! Microphone authorization: `AVCaptureDevice`'s audio permission, read and requested.
//!
//! Flick records through a child process (`rec`), but macOS attributes the child's
//! microphone use to Flick.app, so this is Flick's own grant. The bundle's Info.plist
//! carries `NSMicrophoneUsageDescription` (scripts/bundle.sh); without it macOS refuses.
//!
//! The class is looked up at run time (no objc2-av-foundation crate); the framework is
//! linked through `AVMediaTypeAudio`. Callable from any thread.

use block2::RcBlock;
use objc2::msg_send;
use objc2::runtime::{AnyClass, Bool};
use objc2_foundation::NSString;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeAudio: &'static NSString;
}

/// `AVAuthorizationStatus` for audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Never asked; macOS asks at the first recording (or `request`).
    NotDetermined,
    /// A configuration profile forbids it.
    Restricted,
    Denied,
    Authorized,
}

impl Access {
    fn from_raw(raw: isize) -> Option<Access> {
        match raw {
            0 => Some(Access::NotDetermined),
            1 => Some(Access::Restricted),
            2 => Some(Access::Denied),
            3 => Some(Access::Authorized),
            _ => None,
        }
    }
}

fn device() -> Option<&'static AnyClass> {
    AnyClass::get(c"AVCaptureDevice")
}

fn audio() -> &'static NSString {
    // SAFETY: AVMediaTypeAudio is an immutable framework constant, set at load time.
    unsafe { AVMediaTypeAudio }
}

/// The audio authorization, or `None` if `AVCaptureDevice` is missing or answers a value
/// this build does not know.
pub fn status() -> Option<Access> {
    let class = device()?;
    // SAFETY: `+authorizationStatusForMediaType:` takes an AVMediaType (NSString) and
    // returns an AVAuthorizationStatus (NSInteger); it neither prompts nor blocks.
    let raw: isize = unsafe { msg_send![class, authorizationStatusForMediaType: audio()] };
    Access::from_raw(raw)
}

/// Ask for microphone access (macOS shows its prompt only while `NotDetermined`) and call
/// `done(granted)` on an arbitrary thread. It must only queue work. Tests must not call it.
pub fn request(done: fn(bool)) {
    let Some(class) = device() else {
        done(false);
        return;
    };
    let block = RcBlock::new(move |granted: Bool| done(granted.as_bool()));
    // SAFETY: `+requestAccessForMediaType:completionHandler:` takes an AVMediaType and a
    // `void (^)(BOOL)` block, which it copies.
    let () =
        unsafe { msg_send![class, requestAccessForMediaType: audio(), completionHandler: &*block] };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_statuses_map() {
        let all = [0, 1, 2, 3].map(Access::from_raw);
        assert_eq!(
            all,
            [
                Some(Access::NotDetermined),
                Some(Access::Restricted),
                Some(Access::Denied),
                Some(Access::Authorized)
            ]
        );
        assert_eq!(Access::from_raw(7), None);
        assert_eq!(Access::from_raw(-1), None);
    }

    #[test]
    fn status_answers_without_prompting() {
        assert!(device().is_some(), "AVFoundation is linked");
        assert!(status().is_some());
        // Not called: it may show the system prompt.
        let _: fn(fn(bool)) = request;
    }
}
