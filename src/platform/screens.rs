//! Screen geometry.

use objc2_app_kit::NSScreen;
use objc2_foundation::{NSNumber, NSRect, NSString};

use super::Rect;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDisplayIsBuiltin(display: u32) -> bool;
}

/// One display.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    /// The `CGDirectDisplayID`; stable while the display stays connected.
    pub id: u32,
    /// The display is the Mac's own (laptop panel or iMac screen).
    pub builtin: bool,
    /// The whole display, in Accessibility coordinates.
    pub frame: Rect,
    /// `frame` minus the menu bar and Dock.
    pub visible: Rect,
    /// The display's name ("Built-in Retina Display", "DELL U2723QE").
    pub name: String,
}

/// Usable area (minus menu bar and Dock) of each screen, in Accessibility coordinates.
/// The primary screen comes first.
pub fn visible_areas() -> Vec<Rect> {
    let screens = NSScreen::screens(super::mtm());
    let Some(flip) = flipper(&screens) else { return vec![] };
    screens.iter().map(|s| flip(s.visibleFrame())).collect()
}

/// Every screen, the primary first.
pub fn all() -> Vec<Screen> {
    let screens = NSScreen::screens(super::mtm());
    let Some(flip) = flipper(&screens) else { return vec![] };
    let screen = |s: &NSScreen| Screen {
        id: display_id(s),
        // SAFETY: plain C call; an id that names no display reads false.
        builtin: unsafe { CGDisplayIsBuiltin(display_id(s)) },
        frame: flip(s.frame()),
        visible: flip(s.visibleFrame()),
        name: s.localizedName().to_string(),
    };
    screens.iter().map(|s| screen(&s)).collect()
}

/// The screen's `CGDirectDisplayID` (its `NSScreenNumber`); 0 if it has none.
fn display_id(screen: &NSScreen) -> u32 {
    let number = screen.deviceDescription().objectForKey(&NSString::from_str("NSScreenNumber"));
    number.and_then(|n| n.downcast::<NSNumber>().ok()).map_or(0, |n| n.unsignedIntValue())
}

/// `AppKit` frame (origin bottom-left of the primary screen) to Accessibility coordinates;
/// None without screens.
fn flipper(screens: &objc2_foundation::NSArray<NSScreen>) -> Option<impl Fn(NSRect) -> Rect> {
    let primary_h = screens.iter().next()?.frame().size.height;
    Some(move |f: NSRect| Rect {
        x: f.origin.x,
        y: primary_h - (f.origin.y + f.size.height),
        w: f.size.width,
        h: f.size.height,
    })
}
