//! Escape dismisses every card that is not sticky: a global key monitor for other apps' key
//! events and a local one for Flick's own, installed only while cards show.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::ClassType;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask};
use objc2_foundation::NSObjectProtocol;

use super::{Dismissed, STATE, close, view};
use crate::platform::timer;

const ESCAPE: u16 = 53;

pub(super) fn unmonitor(monitors: &[Retained<AnyObject>]) {
    for monitor in monitors {
        // SAFETY: each monitor came from addGlobal/addLocalMonitorForEvents and is removed
        // once: it was taken out of `State`.
        unsafe { NSEvent::removeMonitor(monitor) };
    }
}

/// Escape: dismiss every card that is not sticky.
fn escape() {
    let ids: Vec<String> = STATE
        .with_borrow(|s| s.cards.iter().filter(|e| !e.opts.sticky).map(|e| e.id.clone()).collect());
    for id in ids {
        close(&id, Dismissed::Escape);
    }
}

/// Escape anywhere: a global monitor for other apps' key events (needs Accessibility;
/// without it, only clicks and timeouts dismiss) and a local one for Flick's own.
pub(super) fn escape_monitors() -> Vec<Retained<AnyObject>> {
    let global = RcBlock::new(|event: NonNull<NSEvent>| {
        // SAFETY: AppKit passes a valid event for the duration of the call.
        if unsafe { event.as_ref() }.keyCode() == ESCAPE {
            timer::after(0.0, escape);
        }
    });
    let local = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: as above.
        let e = unsafe { event.as_ref() };
        // Esc on a card holding the keyboard only gives it back (`focus`).
        let on_card = e
            .window(crate::platform::mtm())
            .is_some_and(|w| w.isKindOfClass(view::CardPanel::class()));
        if e.keyCode() == ESCAPE && !on_card {
            timer::after(0.0, escape);
        }
        event.as_ptr()
    });
    let global =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &global);
    // SAFETY: the block returns the event it was given, unchanged: a valid pointer.
    let local = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &local)
    };
    global.into_iter().chain(local).collect()
}
