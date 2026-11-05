//! Moving, focusing and closing windows: each a job on the window's app thread, so the main
//! thread never waits and apps work in parallel.
//!
//! Frame sets are fire-and-forget. Each app keeps one `Pending` map: a set for a window that
//! is still waiting replaces it, and a window whose newer set arrives while a batch runs is
//! skipped there and applied by the next batch.

use std::collections::HashMap;

use super::observe::Watched;
use super::thread::Inbox;
use super::{WinId, with_session};
use crate::platform::{Rect, ax};

/// While on, apps animate every AX frame change (`AeroSpace` `MacApp.swift:487-499`).
const ENHANCED_UI: &str = "AXEnhancedUserInterface";

/// Frame sets waiting for their app thread, the newest per window.
#[derive(Default)]
pub(super) struct Pending {
    frames: HashMap<WinId, Rect>,
}

impl Pending {
    /// Keep `frame` for `id`, replacing a set still waiting. True when nothing was waiting,
    /// so a job must be sent to apply it.
    pub(super) fn put(&mut self, id: WinId, frame: Rect) -> bool {
        let first = self.frames.is_empty();
        self.frames.insert(id, frame);
        first
    }

    /// Everything waiting, by id; leaves nothing.
    pub(super) fn take(&mut self) -> Vec<(WinId, Rect)> {
        let mut out: Vec<_> = self.frames.drain().collect();
        out.sort_by_key(|(id, _)| *id);
        out
    }

    /// A newer set for `id` arrived since the last `take`.
    pub(super) fn has(&self, id: WinId) -> bool {
        self.frames.contains_key(&id)
    }
}

/// Move and resize windows, each on its app's thread; returns at once. Unknown ids are
/// skipped.
pub fn set_frames(frames: &[(WinId, Rect)]) {
    with_session(|s| {
        for &(id, frame) in frames {
            let Some(app) = s.app_of(id) else { continue };
            let inbox = app.inbox();
            if inbox.frames(|p| p.put(id, frame)) {
                app.send(move |w| apply(w, &inbox));
            }
        }
    });
}

/// App thread: apply the waiting frame sets, with the app's animations off.
fn apply(w: &Watched, inbox: &Inbox) {
    let batch = inbox.frames(Pending::take);
    if batch.is_empty() {
        return;
    }
    let app = w.app();
    let enhanced = ax::bool_attr(app, ENHANCED_UI);
    if enhanced {
        ax::set_bool_attr(app, ENHANCED_UI, false);
    }
    for (id, r) in batch {
        if inbox.frames(|p| p.has(id)) {
            continue;
        }
        let Some(el) = w.window(id) else { continue };
        // Size, position, size (`AeroSpace` `MacApp.swift:474-482`): a window moving to a
        // smaller screen is clamped by its old size, and one moving to a larger screen
        // could not grow before it moved.
        ax::set_size(el, r.w, r.h);
        ax::set_position(el, r.x, r.y);
        ax::set_size(el, r.w, r.h);
    }
    if enhanced {
        ax::set_bool_attr(app, ENHANCED_UI, true);
    }
}

/// Make window `id` its app's main window, raise it, then activate the app (raise first, or
/// the app brings its own front window: `AeroSpace` `MacApp.swift:155-160`).
#[cfg_attr(not(test), expect(dead_code, reason = "tile's focus commands (flick-ac1d) call it"))]
pub fn focus(id: WinId) {
    with_session(|s| {
        let Some(app) = s.app_of(id) else { return };
        app.send(move |w| {
            if let Some(el) = w.window(id) {
                ax::set_bool_attr(el, "AXMain", true);
                ax::perform(el, "AXRaise");
            }
            ax::set_bool_attr(w.app(), "AXFrontmost", true);
        });
    });
}

/// Press window `id`'s close button. The app may still ask to save first.
#[cfg_attr(not(test), expect(dead_code, reason = "tile's close command (flick-c2ea) calls it"))]
pub fn close(id: WinId) {
    with_session(|s| {
        let Some(app) = s.app_of(id) else { return };
        app.send(move |w| {
            let Some(el) = w.window(id) else { return };
            if let Some(button) = ax::copy_attr(el, "AXCloseButton") {
                ax::perform(objc2::rc::Retained::as_ptr(&button).cast(), "AXPress");
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64) -> Rect {
        Rect { x, y: 0.0, w: 10.0, h: 10.0 }
    }

    #[test]
    fn a_newer_set_replaces_a_waiting_one() {
        let mut p = Pending::default();
        assert!(p.put(2, rect(1.0)));
        assert!(!p.put(1, rect(2.0)));
        assert!(!p.put(2, rect(3.0)));
        assert!(p.has(2));
        assert!(!p.has(3));
        assert_eq!(p.take(), [(1, rect(2.0)), (2, rect(3.0))]);
        assert!(!p.has(2));
        assert!(p.take().is_empty());
        // Empty again: the next set needs a new job.
        assert!(p.put(2, rect(4.0)));
    }
}
