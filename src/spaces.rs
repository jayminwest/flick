//! Desktop toggle. There is no public Spaces API, so this goes through apps: activate the most
//! recently used app whose windows are all on another Space, and macOS switches to that Space.

#![expect(
    clippy::undocumented_unsafe_blocks,
    clippy::multiple_unsafe_ops_per_block,
    reason = "unsafe moves to src/platform with SAFETY comments in flick-ee5b"
)]

use std::cell::RefCell;
use std::collections::HashSet;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSNotification, NSNumber, NSObjectProtocol, NSString,
};

const MAX_RECENT: usize = 32;

thread_local! {
    /// Pids of activated apps, most recent first.
    static RECENT: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static OBSERVER: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> = const { RefCell::new(None) };
}

fn record(pid: i32) {
    RECENT.with(|r| {
        let mut r = r.borrow_mut();
        r.retain(|&p| p != pid);
        r.insert(0, pid);
        r.truncate(MAX_RECENT);
    });
}

/// Pids of activated apps, most recent first.
pub fn recent() -> Vec<i32> {
    RECENT.with(|r| r.borrow().clone())
}

pub fn frontmost_pid() -> Option<i32> {
    NSWorkspace::sharedWorkspace().frontmostApplication().map(|a| a.processIdentifier())
}

/// Start tracking app activations.
pub fn init() {
    if let Some(pid) = frontmost_pid() {
        record(pid);
    }
    let block = RcBlock::new(|_n: NonNull<NSNotification>| {
        if let Some(pid) = frontmost_pid() {
            record(pid);
        }
    });
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            None,
            &block,
        )
    };
    OBSERVER.with(|o| *o.borrow_mut() = Some(observer));
}

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
    // CFArray of CFDictionary, toll-free bridged; "Copy" returns +1.
    let Some(list) = (unsafe { Retained::from_raw(CGWindowListCopyWindowInfo(option, 0)) }) else {
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

/// Pick the app (other than `current`) with windows, none of them on this Space: the most recent
/// one in `recent`, else the first in `fallback` (history is empty right after Flick starts).
fn pick(
    recent: &[i32],
    fallback: &[i32],
    current: Option<i32>,
    here: &HashSet<i32>,
    anywhere: &HashSet<i32>,
) -> Option<i32> {
    recent
        .iter()
        .chain(fallback)
        .copied()
        .find(|pid| Some(*pid) != current && anywhere.contains(pid) && !here.contains(pid))
}

/// Activate `app` the way a Dock click does, which also switches to its Space.
/// Needs no Accessibility permission.
pub fn open_app(app: &NSRunningApplication) -> bool {
    let Some(url) = app.bundleURL() else { return false };
    NSWorkspace::sharedWorkspace().openURL(&url)
}

pub fn toggle() -> Result<(), &'static str> {
    // Regular (Dock) apps only: menu-bar apps keep hidden windows but have no desktop to switch to.
    let regular = |pid: &i32| {
        NSRunningApplication::runningApplicationWithProcessIdentifier(*pid)
            .is_some_and(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular)
    };
    let recent: Vec<i32> = RECENT.with(|r| r.borrow().iter().copied().filter(regular).collect());
    let fallback: Vec<i32> = NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|a| !a.isHidden())
        .map(|a| a.processIdentifier())
        .filter(regular)
        .collect();
    let pid = pick(&recent, &fallback, frontmost_pid(), &window_pids(true), &window_pids(false))
        .ok_or("No app on another desktop")?;
    let app =
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid).ok_or("App quit")?;
    if !open_app(&app) {
        return Err("App has no bundle");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_most_recent_app_on_another_space() {
        let here: HashSet<i32> = [1, 2].into();
        let anywhere: HashSet<i32> = [1, 2, 3, 4].into();
        // 2 is on this Space, 5 has no windows, so 3 wins over the older 4.
        assert_eq!(pick(&[1, 2, 5, 3, 4], &[], Some(1), &here, &anywhere), Some(3));
        assert_eq!(pick(&[1, 2], &[], Some(1), &here, &anywhere), None);
        // Empty history falls back to any app on another Space.
        assert_eq!(pick(&[1], &[2, 4], Some(1), &here, &anywhere), Some(4));
    }
}
