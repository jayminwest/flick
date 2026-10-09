//! Ink: annotation on screenshots and drawing on the screen. `model` is plain Rust (shapes,
//! undo, key map, geometry) with a 100% coverage floor; `view` is the `AppKit` canvas that
//! draws it, `editor` the annotation window, and `overlay` draws on the screen itself.

pub mod editor;
pub mod model;
pub mod overlay;
pub mod view;
