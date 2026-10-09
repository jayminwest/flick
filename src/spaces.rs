//! Desktop toggle. There is no public Spaces API, so this goes through apps: activate the most
//! recently used app whose windows are all on another Space, and macOS switches to that Space.

use std::cell::RefCell;
use std::collections::HashSet;

pub use crate::platform::workspace::frontmost_pid;
use crate::platform::{spaces, workspace};

const MAX_RECENT: usize = 32;

thread_local! {
    /// Pids of activated apps, most recent first.
    static RECENT: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
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

/// Start tracking app activations.
pub fn init() {
    if let Some(pid) = frontmost_pid() {
        record(pid);
    }
    workspace::on_app_activated(|| {
        if let Some(pid) = frontmost_pid() {
            record(pid);
        }
    });
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

pub fn toggle() -> Result<(), &'static str> {
    // Regular (Dock) apps only: menu-bar apps keep hidden windows but have no desktop to switch to.
    let regular = |pid: &i32| workspace::is_regular(*pid);
    let recent: Vec<i32> = RECENT.with(|r| r.borrow().iter().copied().filter(regular).collect());
    let fallback: Vec<i32> = workspace::unhidden_app_pids().into_iter().filter(regular).collect();
    let current = frontmost_pid();
    let (here, anywhere) = (spaces::window_pids(true), spaces::window_pids(false));
    let pid =
        pick(&recent, &fallback, current, &here, &anywhere).ok_or("No app on another desktop")?;
    if !workspace::open_app(pid)? {
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
