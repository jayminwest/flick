//! Screen geometry.

use objc2_app_kit::NSScreen;
use objc2_foundation::NSRect;

use super::Rect;

/// Usable area (minus menu bar and Dock) of each screen, in Accessibility coordinates.
/// The primary screen comes first.
pub fn visible_areas() -> Vec<Rect> {
    let screens = NSScreen::screens(super::mtm());
    let Some(primary_h) = screens.iter().next().map(|s| s.frame().size.height) else {
        return vec![];
    };
    let flip = |f: NSRect| Rect {
        x: f.origin.x,
        y: primary_h - (f.origin.y + f.size.height),
        w: f.size.width,
        h: f.size.height,
    };
    screens.iter().map(|s| flip(s.visibleFrame())).collect()
}
