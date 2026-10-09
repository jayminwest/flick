//! Window management through the Accessibility API.
//!
//! Geometry uses the Accessibility coordinate space: origin at the top-left of the
//! primary screen, y grows downward.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_foundation::{NSDictionary, NSNumber, NSPoint, NSRect, NSSize, NSString};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Equal within a few points; apps round their frames.
    fn near(&self, o: Rect) -> bool {
        [self.x - o.x, self.y - o.y, self.w - o.w, self.h - o.h].iter().all(|d| d.abs() <= 4.0)
    }

    fn contains(&self, (px, py): (f64, f64)) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    FirstThird,
    CenterThird,
    LastThird,
    FirstTwoThirds,
    LastTwoThirds,
    Maximize,
    AlmostMaximize,
    Center,
    NextDisplay,
    PreviousDisplay,
}

impl WindowAction {
    pub const ALL: [WindowAction; 18] = [
        Self::LeftHalf,
        Self::RightHalf,
        Self::TopHalf,
        Self::BottomHalf,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::FirstThird,
        Self::CenterThird,
        Self::LastThird,
        Self::FirstTwoThirds,
        Self::LastTwoThirds,
        Self::Maximize,
        Self::AlmostMaximize,
        Self::Center,
        Self::NextDisplay,
        Self::PreviousDisplay,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::LeftHalf => "Left Half",
            Self::RightHalf => "Right Half",
            Self::TopHalf => "Top Half",
            Self::BottomHalf => "Bottom Half",
            Self::TopLeft => "Top Left Quarter",
            Self::TopRight => "Top Right Quarter",
            Self::BottomLeft => "Bottom Left Quarter",
            Self::BottomRight => "Bottom Right Quarter",
            Self::FirstThird => "First Third",
            Self::CenterThird => "Center Third",
            Self::LastThird => "Last Third",
            Self::FirstTwoThirds => "First Two Thirds",
            Self::LastTwoThirds => "Last Two Thirds",
            Self::Maximize => "Maximize",
            Self::AlmostMaximize => "Almost Maximize",
            Self::Center => "Center",
            Self::NextDisplay => "Next Display",
            Self::PreviousDisplay => "Previous Display",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::LeftHalf => "rectangle.lefthalf.inset.filled",
            Self::RightHalf => "rectangle.righthalf.inset.filled",
            Self::TopHalf => "rectangle.tophalf.inset.filled",
            Self::BottomHalf => "rectangle.bottomhalf.inset.filled",
            Self::TopLeft => "rectangle.inset.topleft.filled",
            Self::TopRight => "rectangle.inset.topright.filled",
            Self::BottomLeft => "rectangle.inset.bottomleft.filled",
            Self::BottomRight => "rectangle.inset.bottomright.filled",
            Self::FirstThird | Self::FirstTwoThirds => "rectangle.leadingthird.inset.filled",
            Self::CenterThird => "rectangle.center.inset.filled",
            Self::LastThird | Self::LastTwoThirds => "rectangle.trailingthird.inset.filled",
            Self::Maximize => "rectangle.inset.filled",
            Self::AlmostMaximize => "rectangle.dashed",
            Self::Center => "rectangle.center.inset.filled",
            Self::NextDisplay | Self::PreviousDisplay => "display.2",
        }
    }

    /// Config name: the title in kebab case, e.g. "left-half".
    pub fn slug(self) -> String {
        self.title().to_lowercase().replace(' ', "-")
    }

    pub fn from_slug(slug: &str) -> Option<WindowAction> {
        Self::ALL.into_iter().find(|a| a.slug() == slug)
    }

    /// Target frame within the screen's usable `area`, given the window's `current` frame.
    /// Display moves are handled by `apply`, so here they keep the frame.
    pub fn target(self, a: Rect, current: Rect) -> Rect {
        let (hw, hh, tw) = (a.w / 2.0, a.h / 2.0, a.w / 3.0);
        let r = |x: f64, y: f64, w: f64, h: f64| Rect { x: a.x + x, y: a.y + y, w, h };
        match self {
            Self::LeftHalf => r(0.0, 0.0, hw, a.h),
            Self::RightHalf => r(hw, 0.0, hw, a.h),
            Self::TopHalf => r(0.0, 0.0, a.w, hh),
            Self::BottomHalf => r(0.0, hh, a.w, hh),
            Self::TopLeft => r(0.0, 0.0, hw, hh),
            Self::TopRight => r(hw, 0.0, hw, hh),
            Self::BottomLeft => r(0.0, hh, hw, hh),
            Self::BottomRight => r(hw, hh, hw, hh),
            Self::FirstThird => r(0.0, 0.0, tw, a.h),
            Self::CenterThird => r(tw, 0.0, tw, a.h),
            Self::LastThird => r(2.0 * tw, 0.0, tw, a.h),
            Self::FirstTwoThirds => r(0.0, 0.0, 2.0 * tw, a.h),
            Self::LastTwoThirds => r(tw, 0.0, 2.0 * tw, a.h),
            Self::Maximize => a,
            Self::AlmostMaximize => r(a.w * 0.05, a.h * 0.05, a.w * 0.9, a.h * 0.9),
            Self::Center => {
                let (w, h) = (current.w.min(a.w), current.h.min(a.h));
                r((a.w - w) / 2.0, (a.h - h) / 2.0, w, h)
            }
            Self::NextDisplay | Self::PreviousDisplay => current,
        }
    }

    /// Like `target`, but repeating a half cycles its size through 1/2, 2/3, 1/3 (as Rectangle does).
    pub fn cycled(self, a: Rect, current: Rect) -> Rect {
        let frame = |f: f64| {
            let r = match self {
                Self::LeftHalf => Rect { w: a.w * f, ..a },
                Self::RightHalf => Rect { x: a.x + a.w * (1.0 - f), w: a.w * f, ..a },
                Self::TopHalf => Rect { h: a.h * f, ..a },
                Self::BottomHalf => Rect { y: a.y + a.h * (1.0 - f), h: a.h * f, ..a },
                _ => return None,
            };
            Some(Rect { x: r.x.round(), y: r.y.round(), w: r.w.round(), h: r.h.round() })
        };
        const SIZES: [f64; 3] = [1.0 / 2.0, 2.0 / 3.0, 1.0 / 3.0];
        let Some(_) = frame(SIZES[0]) else { return self.target(a, current) };
        let at = SIZES.iter().position(|&f| frame(f).is_some_and(|r| r.near(current)));
        frame(SIZES[at.map_or(0, |i| (i + 1) % SIZES.len())]).unwrap()
    }
}

/// Map `current` from screen area `from` to the same relative spot on `to`.
fn move_between(current: Rect, from: Rect, to: Rect) -> Rect {
    let w = current.w.min(to.w);
    let h = current.h.min(to.h);
    let fx = if from.w > current.w { (current.x - from.x) / (from.w - current.w) } else { 0.0 };
    let fy = if from.h > current.h { (current.y - from.y) / (from.h - current.h) } else { 0.0 };
    Rect {
        x: to.x + fx.clamp(0.0, 1.0) * (to.w - w),
        y: to.y + fy.clamp(0.0, 1.0) * (to.h - h),
        w,
        h,
    }
}

// --- Accessibility FFI ---

type CFTypeRef = *const c_void;
type AXUIElementRef = *const c_void;

const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;
const CG_HID_EVENT_TAP: u32 = 0;
const CG_EVENT_FLAG_COMMAND: u64 = 1 << 20;
const KEY_V: u16 = 9;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(el: AXUIElementRef, attr: CFTypeRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(el: AXUIElementRef, attr: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(v: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn CGEventCreateKeyboardEvent(source: CFTypeRef, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
}

/// Owned CoreFoundation reference.
struct Cf(CFTypeRef);

impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

// NSString is toll-free bridged to CFString.
fn cfstr(s: &NSString) -> CFTypeRef {
    s as *const NSString as CFTypeRef
}

/// Whether Flick may control other apps; if not, show macOS's permission dialog once per launch.
pub fn ensure_trusted() -> bool {
    static PROMPTED: AtomicBool = AtomicBool::new(false);
    is_trusted(false) || (!PROMPTED.swap(true, Ordering::Relaxed) && is_trusted(true))
}

/// Whether Flick may control other apps. With `prompt`, macOS shows its permission dialog.
fn is_trusted(prompt: bool) -> bool {
    let key = NSString::from_str("AXTrustedCheckOptionPrompt");
    let value = NSNumber::new_bool(prompt);
    let options = NSDictionary::from_retained_objects(&[&*key], &[value]);
    unsafe { AXIsProcessTrustedWithOptions(Retained::as_ptr(&options) as CFTypeRef) }
}

/// Press Cmd+V in the frontmost app.
pub fn send_paste() {
    unsafe {
        for down in [true, false] {
            let event = CGEventCreateKeyboardEvent(ptr::null(), KEY_V, down);
            if event.is_null() {
                return;
            }
            CGEventSetFlags(event, CG_EVENT_FLAG_COMMAND);
            CGEventPost(CG_HID_EVENT_TAP, event);
            CFRelease(event);
        }
    }
}

/// Activate app `pid` through Accessibility. Returns false if that failed.
pub fn make_frontmost(pid: i32) -> bool {
    if !is_trusted(false) {
        return false;
    }
    let app_el = Cf(unsafe { AXUIElementCreateApplication(pid) });
    let attr = NSString::from_str("AXFrontmost");
    // NSNumber's YES is the kCFBooleanTrue singleton.
    let yes = NSNumber::new_bool(true);
    let value = Retained::as_ptr(&yes) as CFTypeRef;
    unsafe { AXUIElementSetAttributeValue(app_el.0, cfstr(&attr), value) == 0 }
}

fn focused_window() -> Option<Cf> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = app.processIdentifier();
    let app_el = Cf(unsafe { AXUIElementCreateApplication(pid) });
    let attr = NSString::from_str("AXFocusedWindow");
    let mut win: CFTypeRef = ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(app_el.0, cfstr(&attr), &mut win) };
    (err == 0 && !win.is_null()).then(|| Cf(win))
}

fn get_value<T: Default>(win: &Cf, attr: &str, kind: u32) -> Option<T> {
    let attr = NSString::from_str(attr);
    let mut value: CFTypeRef = ptr::null();
    if unsafe { AXUIElementCopyAttributeValue(win.0, cfstr(&attr), &mut value) } != 0 || value.is_null() {
        return None;
    }
    let value = Cf(value);
    let mut out = T::default();
    unsafe { AXValueGetValue(value.0, kind, &mut out as *mut T as *mut c_void) }.then_some(out)
}

fn set_value<T>(win: &Cf, attr: &str, kind: u32, v: &T) {
    let attr = NSString::from_str(attr);
    let value = Cf(unsafe { AXValueCreate(kind, v as *const T as *const c_void) });
    unsafe { AXUIElementSetAttributeValue(win.0, cfstr(&attr), value.0) };
}

fn window_frame(win: &Cf) -> Option<Rect> {
    let p: NSPoint = get_value(win, "AXPosition", AX_VALUE_CGPOINT)?;
    let s: NSSize = get_value(win, "AXSize", AX_VALUE_CGSIZE)?;
    Some(Rect { x: p.x, y: p.y, w: s.width, h: s.height })
}

fn set_window_frame(win: &Cf, r: Rect) {
    let pos = NSPoint::new(r.x, r.y);
    let size = NSSize::new(r.w, r.h);
    // Position, size, position: moving across screens can clamp the size otherwise.
    set_value(win, "AXPosition", AX_VALUE_CGPOINT, &pos);
    set_value(win, "AXSize", AX_VALUE_CGSIZE, &size);
    set_value(win, "AXPosition", AX_VALUE_CGPOINT, &pos);
}

/// Usable area (minus menu bar and Dock) of each screen, in Accessibility coordinates.
fn screen_areas(mtm: MainThreadMarker) -> Vec<Rect> {
    let screens = NSScreen::screens(mtm);
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

pub fn apply(action: WindowAction, mtm: MainThreadMarker) -> Result<(), &'static str> {
    if !ensure_trusted() {
        return Err("Flick needs Accessibility permission");
    }
    let win = focused_window().ok_or("No focused window")?;
    let current = window_frame(&win).ok_or("Can't read window frame")?;
    let areas = screen_areas(mtm);
    let index = areas.iter().position(|a| a.contains(current.center())).unwrap_or(0);
    let area = *areas.get(index).ok_or("No screens")?;
    let n = areas.len();
    let target = match action {
        WindowAction::NextDisplay => move_between(current, area, areas[(index + 1) % n]),
        WindowAction::PreviousDisplay => move_between(current, area, areas[(index + n - 1) % n]),
        _ => action.cycled(area, current),
    };
    set_window_frame(&win, target);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0.0, y: 25.0, w: 1200.0, h: 800.0 };
    const WIN: Rect = Rect { x: 100.0, y: 100.0, w: 400.0, h: 300.0 };

    #[test]
    fn halves_and_quarters() {
        assert_eq!(WindowAction::LeftHalf.target(AREA, WIN), Rect { x: 0.0, y: 25.0, w: 600.0, h: 800.0 });
        assert_eq!(WindowAction::BottomRight.target(AREA, WIN), Rect { x: 600.0, y: 425.0, w: 600.0, h: 400.0 });
        assert_eq!(WindowAction::LastTwoThirds.target(AREA, WIN), Rect { x: 400.0, y: 25.0, w: 800.0, h: 800.0 });
    }

    #[test]
    fn repeated_halves_cycle_sizes() {
        let half = WindowAction::RightHalf.cycled(AREA, WIN);
        assert_eq!(half, Rect { x: 600.0, y: 25.0, w: 600.0, h: 800.0 });
        let two_thirds = WindowAction::RightHalf.cycled(AREA, half);
        assert_eq!(two_thirds, Rect { x: 400.0, y: 25.0, w: 800.0, h: 800.0 });
        let third = WindowAction::RightHalf.cycled(AREA, Rect { w: 799.0, ..two_thirds });
        assert_eq!(third, Rect { x: 800.0, y: 25.0, w: 400.0, h: 800.0 });
        assert_eq!(WindowAction::RightHalf.cycled(AREA, third), half);
        assert_eq!(WindowAction::Maximize.cycled(AREA, AREA), AREA);
    }

    #[test]
    fn slugs_round_trip() {
        for a in WindowAction::ALL {
            assert_eq!(WindowAction::from_slug(&a.slug()), Some(a));
        }
        assert_eq!(WindowAction::LeftHalf.slug(), "left-half");
    }

    #[test]
    fn center_keeps_size() {
        assert_eq!(WindowAction::Center.target(AREA, WIN), Rect { x: 400.0, y: 275.0, w: 400.0, h: 300.0 });
    }

    #[test]
    fn next_display_keeps_relative_position() {
        let to = Rect { x: 1200.0, y: 0.0, w: 2400.0, h: 1600.0 };
        let left = Rect { x: 0.0, y: 25.0, w: 400.0, h: 300.0 };
        let moved = move_between(left, AREA, to);
        assert_eq!((moved.x, moved.y, moved.w), (1200.0, 0.0, 400.0));
    }
}
