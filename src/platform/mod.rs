//! The macOS layer: the only place with `unsafe`, objc2, `AppKit`, `CoreFoundation`,
//! Accessibility, `CoreGraphics` or Carbon. Everything here is a safe, Flick-shaped function;
//! callers never see Objective-C objects, CF types or selectors.
//!
//! `AppKit` runs on the main thread, and so does every function here.

pub mod app;
pub mod ax;
pub mod events;
pub mod files;
pub mod hotkeys;
pub mod keytap;
pub mod panel;
pub mod pasteboard;
pub mod screens;
pub mod spaces;
pub mod timer;
pub mod workspace;

use objc2::MainThreadMarker;

/// A frame in the Accessibility coordinate space: origin at the top-left of the primary
/// screen, y grows downward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[expect(clippy::expect_used, reason = "AppKit callbacks only ever run on the main thread")]
fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("Flick UI runs on the main thread")
}
