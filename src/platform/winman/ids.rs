//! Window ids: the window server's `CGWindowID` behind an AX window element, and what the
//! window server says about one.
//!
//! `_AXUIElementGetWindow` is private (`AeroSpace` `Sources/PrivateApi/include/private.h`), but
//! stable since 10.x and used by `AeroSpace`, yabai and `AltTab`. This file is its only caller.

use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

use super::WinId;
use crate::platform::ax::CFTypeRef;

/// `kCGNullWindowID`.
const NULL_WINDOW: u32 = 0;
/// `kCGWindowListOptionIncludingWindow`: just the window named.
const INCLUDING_WINDOW: u32 = 1 << 3;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn _AXUIElementGetWindow(el: CFTypeRef, out: *mut u32) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(
        option: u32,
        relative_to: u32,
    ) -> *mut NSArray<NSDictionary<NSString>>;
}

/// The window id of AX element `el`; None when it is not a window (an error, or the null id,
/// as for Finder's desktop).
pub(super) fn window_id(el: CFTypeRef) -> Option<WinId> {
    let mut id = NULL_WINDOW;
    // SAFETY: `el` is a live AX element for the call; `id` is a valid out pointer.
    let err = unsafe { _AXUIElementGetWindow(el, &mut id) };
    checked(err, id)
}

/// Window `id`'s level (`kCGWindowLayer`); None when the window server does not know it.
/// Public `CoreGraphics`, safe on any thread; needs no Screen Recording permission.
pub(super) fn level(id: WinId) -> Option<i32> {
    // SAFETY: plain C call with a valid option flag and window id.
    let raw = unsafe { CGWindowListCopyWindowInfo(INCLUDING_WINDOW, id) };
    // SAFETY: a CFArray of CFDictionary, toll-free bridged to NSArray of NSDictionary; "Copy"
    // returns +1, which Retained takes over. Null maps to None.
    let list = unsafe { Retained::from_raw(raw) }?;
    let info = list.firstObject()?;
    let layer = info.objectForKey(&NSString::from_str("kCGWindowLayer"))?;
    Some(layer.downcast::<NSNumber>().ok()?.intValue())
}

/// An id only for a successful call that names a window.
fn checked(err: i32, id: u32) -> Option<WinId> {
    (err == 0 && id != NULL_WINDOW).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::ax;

    #[test]
    fn errors_and_the_null_id_are_not_windows() {
        assert_eq!(checked(0, 42), Some(42));
        assert_eq!(checked(0, NULL_WINDOW), None);
        assert_eq!(checked(-25202, 42), None);
    }

    #[test]
    fn an_app_element_is_not_a_window() {
        let app = ax::app_element(std::process::id() as i32);
        assert_eq!(window_id(app.0), None);
    }

    #[test]
    fn the_null_window_has_no_level() {
        assert_eq!(level(NULL_WINDOW), None);
    }
}
