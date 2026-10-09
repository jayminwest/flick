//! Main run loop timers.

use block2::RcBlock;
use objc2_foundation::NSTimer;

/// Run `f` on the main run loop after `secs`.
pub fn after(secs: f64, f: impl Fn() + 'static) {
    let block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| f());
    // SAFETY: the block is 'static and takes the `NSTimer` argument the timer passes; the
    // run loop retains the scheduled timer, which retains the block.
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(secs, false, &block) };
}

/// Run `f` on the main run loop every `secs`.
pub fn every(secs: f64, f: impl Fn() + 'static) {
    let block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| f());
    // SAFETY: as in `after`.
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(secs, true, &block) };
}
