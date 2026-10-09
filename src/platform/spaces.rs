//! Which apps own windows on which Space, through the CoreGraphics window list.

use std::collections::HashSet;

use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

const CG_WINDOW_LIST_ALL: u32 = 0;
const CG_WINDOW_LIST_ON_SCREEN_ONLY: u32 = 1;
const CG_WINDOW_LIST_EXCLUDE_DESKTOP: u32 = 16;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(
        option: u32,
        relative_to: u32,
    ) -> *mut NSArray<NSDictionary<NSString>>;
}

/// Pids owning normal (layer 0) windows: on the current Space only, or anywhere.
/// Reading pids and layers needs no Screen Recording permission.
pub fn window_pids(current_space_only: bool) -> HashSet<i32> {
    let option = CG_WINDOW_LIST_EXCLUDE_DESKTOP
        | if current_space_only { CG_WINDOW_LIST_ON_SCREEN_ONLY } else { CG_WINDOW_LIST_ALL };
    // SAFETY: plain C call with valid option flags and window id 0 (no relative window).
    let raw = unsafe { CGWindowListCopyWindowInfo(option, 0) };
    // SAFETY: the result is a CFArray of CFDictionary, toll-free bridged to NSArray of
    // NSDictionary; "Copy" returns +1, which Retained takes over. Null maps to None.
    let Some(list) = (unsafe { Retained::from_raw(raw) }) else {
        return HashSet::new();
    };
    let (pid_key, layer_key) =
        (NSString::from_str("kCGWindowOwnerPID"), NSString::from_str("kCGWindowLayer"));
    let number = |info: &NSDictionary<NSString>, key: &NSString| {
        info.objectForKey(key).and_then(|v| v.downcast::<NSNumber>().ok()).map(|n| n.intValue())
    };
    list.iter()
        .filter(|info| number(info, &layer_key) == Some(0))
        .filter_map(|info| number(&info, &pid_key))
        .collect()
}
