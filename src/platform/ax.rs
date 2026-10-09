//! Other apps' windows through the Accessibility API, and synthetic key presses.
//!
//! Geometry uses the Accessibility coordinate space (see `Rect`).

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSRunningApplication;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSPoint, NSSize, NSString};

use super::Rect;

pub(super) type CFTypeRef = *const c_void;
type AXUIElementRef = *const c_void;

const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;
const CG_HID_EVENT_TAP: u32 = 0;
const CG_EVENT_FLAG_COMMAND: u64 = 1 << 20;
const KEY_V: u16 = 9;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        el: AXUIElementRef,
        attr: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(el: AXUIElementRef, attr: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(v: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn AXUIElementSetMessagingTimeout(el: AXUIElementRef, seconds: f32) -> i32;
    fn AXUIElementPerformAction(el: AXUIElementRef, action: CFTypeRef) -> i32;
    fn CGEventCreateKeyboardEvent(source: CFTypeRef, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
}

/// Owned CoreFoundation reference.
pub(super) struct Cf(pub(super) CFTypeRef);

impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `Cf` owns one +1 reference to a live, non-null CF object.
            unsafe { CFRelease(self.0) }
        }
    }
}

// NSString is toll-free bridged to CFString.
pub(super) fn cfstr(s: &NSString) -> CFTypeRef {
    s as *const NSString as CFTypeRef
}

/// The Accessibility element for app `pid` (valid even if no such app runs; calls then fail).
pub(super) fn app_element(pid: i32) -> Cf {
    // SAFETY: plain C call; it returns a +1 reference (or null), which `Cf` releases.
    Cf(unsafe { AXUIElementCreateApplication(pid) })
}

/// Whether Flick may control other apps; if not, show macOS's permission dialog once per launch.
pub fn ensure_trusted() -> bool {
    static PROMPTED: AtomicBool = AtomicBool::new(false);
    is_trusted(false) || (!PROMPTED.swap(true, Ordering::Relaxed) && is_trusted(true))
}

/// Whether Flick may control other apps. With `prompt`, macOS shows its permission dialog.
pub fn is_trusted(prompt: bool) -> bool {
    let key = NSString::from_str("AXTrustedCheckOptionPrompt");
    let value = NSNumber::new_bool(prompt);
    let options = NSDictionary::from_retained_objects(&[&*key], &[value]);
    // SAFETY: NSDictionary is toll-free bridged to CFDictionary and outlives the call.
    unsafe { AXIsProcessTrustedWithOptions(Retained::as_ptr(&options) as CFTypeRef) }
}

/// Press Cmd+V in the frontmost app.
pub fn send_paste() {
    for down in [true, false] {
        // SAFETY: a null source is allowed; the result is a +1 reference or null.
        let event = unsafe { CGEventCreateKeyboardEvent(ptr::null(), KEY_V, down) };
        if event.is_null() {
            return;
        }
        // SAFETY: `event` is a live, non-null CGEvent.
        unsafe { CGEventSetFlags(event, CG_EVENT_FLAG_COMMAND) };
        // SAFETY: as above; posting does not consume the reference.
        unsafe { CGEventPost(CG_HID_EVENT_TAP, event) };
        // SAFETY: releases the +1 reference from CGEventCreateKeyboardEvent, used no more.
        unsafe { CFRelease(event) };
    }
}

/// Activate app `pid` through Accessibility. Returns false if that failed.
pub fn make_frontmost(pid: i32) -> bool {
    if !is_trusted(false) {
        return false;
    }
    let app_el = app_element(pid);
    let attr = NSString::from_str("AXFrontmost");
    // NSNumber's YES is the kCFBooleanTrue singleton.
    let yes = NSNumber::new_bool(true);
    let value = Retained::as_ptr(&yes) as CFTypeRef;
    // SAFETY: the element, attribute name and value are live CF objects for the whole call.
    unsafe { AXUIElementSetAttributeValue(app_el.0, cfstr(&attr), value) == 0 }
}

/// The focused window of the frontmost app.
pub struct FocusedWindow(Cf);

pub fn focused_window() -> Option<FocusedWindow> {
    let pid = super::workspace::frontmost_pid()?;
    let app_el = app_element(pid);
    let attr = NSString::from_str("AXFocusedWindow");
    let mut win: CFTypeRef = ptr::null();
    // SAFETY: live element and attribute; `win` receives a +1 reference on success.
    let err = unsafe { AXUIElementCopyAttributeValue(app_el.0, cfstr(&attr), &mut win) };
    (err == 0 && !win.is_null()).then(|| FocusedWindow(Cf(win)))
}

/// The title of app `pid`'s focused window. None without Accessibility permission, without a
/// focused window, or when the window has no title.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-a30b"))]
pub fn focused_window_title(pid: i32) -> Option<String> {
    let app_el = app_element(pid);
    if app_el.0.is_null() {
        return None;
    }
    // A hung app must not stall the caller.
    // SAFETY: `app_el` is a live element.
    unsafe { AXUIElementSetMessagingTimeout(app_el.0, 0.25) };
    let win = copy_attr(app_el.0, "AXFocusedWindow")?;
    string_attr(Retained::as_ptr(&win) as CFTypeRef, "AXTitle").filter(|t| !t.is_empty())
}

fn get_value<T: Default>(win: &Cf, attr: &str, kind: u32) -> Option<T> {
    let attr = NSString::from_str(attr);
    let mut value: CFTypeRef = ptr::null();
    // SAFETY: live element and attribute; `value` receives a +1 reference on success.
    if unsafe { AXUIElementCopyAttributeValue(win.0, cfstr(&attr), &mut value) } != 0
        || value.is_null()
    {
        return None;
    }
    let value = Cf(value);
    let mut out = T::default();
    // SAFETY: callers pass the `T` (CGPoint or CGSize) that matches `kind`.
    unsafe { AXValueGetValue(value.0, kind, &mut out as *mut T as *mut c_void) }.then_some(out)
}

fn set_value<T>(win: &Cf, attr: &str, kind: u32, v: &T) {
    let attr = NSString::from_str(attr);
    // SAFETY: callers pass the `T` (CGPoint or CGSize) that matches `kind`; +1 result.
    let value = Cf(unsafe { AXValueCreate(kind, v as *const T as *const c_void) });
    // SAFETY: element, attribute and value are live CF objects for the whole call.
    unsafe { AXUIElementSetAttributeValue(win.0, cfstr(&attr), value.0) };
}

impl FocusedWindow {
    pub fn frame(&self) -> Option<Rect> {
        let p: NSPoint = get_value(&self.0, "AXPosition", AX_VALUE_CGPOINT)?;
        let s: NSSize = get_value(&self.0, "AXSize", AX_VALUE_CGSIZE)?;
        Some(Rect { x: p.x, y: p.y, w: s.width, h: s.height })
    }

    pub fn set_frame(&self, r: Rect) {
        let pos = NSPoint::new(r.x, r.y);
        let size = NSSize::new(r.w, r.h);
        // Position, size, position: moving across screens can clamp the size otherwise.
        set_value(&self.0, "AXPosition", AX_VALUE_CGPOINT, &pos);
        set_value(&self.0, "AXSize", AX_VALUE_CGSIZE, &size);
        set_value(&self.0, "AXPosition", AX_VALUE_CGPOINT, &pos);
    }

    pub fn minimize(&self) {
        set_bool_attr(self.0.0, "AXMinimized", true);
    }
}

// --- Window switcher ---

/// A standard window of some app, kept to focus it later.
pub struct AxWindow {
    /// None when the window has no title.
    pub title: Option<String>,
    pub minimized: bool,
    element: Retained<AnyObject>,
}

/// Copy an attribute as an Objective-C object (CF types are toll-free bridged).
pub(super) fn copy_attr(el: CFTypeRef, attr: &str) -> Option<Retained<AnyObject>> {
    let attr = NSString::from_str(attr);
    let mut value: CFTypeRef = ptr::null();
    // SAFETY: live element and attribute; `value` receives a +1 reference on success.
    if unsafe { AXUIElementCopyAttributeValue(el, cfstr(&attr), &mut value) } != 0 {
        return None;
    }
    // SAFETY: "Copy" returns +1, which Retained takes over; null maps to None.
    unsafe { Retained::from_raw(value as *mut AnyObject) }
}

fn string_attr(el: CFTypeRef, attr: &str) -> Option<String> {
    copy_attr(el, attr)?.downcast::<NSString>().ok().map(|s| s.to_string())
}

fn bool_attr(el: CFTypeRef, attr: &str) -> bool {
    copy_attr(el, attr).and_then(|v| v.downcast::<NSNumber>().ok()).is_some_and(|n| n.boolValue())
}

fn set_bool_attr(el: CFTypeRef, attr: &str, value: bool) {
    let attr = NSString::from_str(attr);
    let value = NSNumber::new_bool(value);
    // SAFETY: element, attribute and value are live CF objects for the whole call.
    unsafe {
        AXUIElementSetAttributeValue(el, cfstr(&attr), Retained::as_ptr(&value) as CFTypeRef)
    };
}

/// App `pid`'s standard windows (on the current desktop only), front to back.
pub fn standard_windows(pid: i32) -> Vec<AxWindow> {
    let app_el = app_element(pid);
    // A hung app must not stall the switcher.
    // SAFETY: `app_el` is a live element.
    unsafe { AXUIElementSetMessagingTimeout(app_el.0, 0.25) };

    let mut out = Vec::new();
    let windows = copy_attr(app_el.0, "AXWindows").and_then(|w| w.downcast::<NSArray>().ok());
    for win in windows.iter().flat_map(|w| w.iter()) {
        let el = Retained::as_ptr(&win) as CFTypeRef;
        if string_attr(el, "AXSubrole").as_deref() != Some("AXStandardWindow") {
            continue;
        }
        let title = string_attr(el, "AXTitle");
        out.push(AxWindow { title, minimized: bool_attr(el, "AXMinimized"), element: win });
    }
    out
}

/// Raise `window` of app `pid` and activate the app (switching desktops if needed). Without a
/// window, just activate the app. `minimized`: the window was minimized when listed.
pub fn focus(pid: i32, window: Option<&AxWindow>, minimized: bool) {
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return;
    };
    if app.isHidden() {
        app.unhide();
    }
    let Some(window) = window else {
        super::workspace::open_running(&app);
        return;
    };
    let el = Retained::as_ptr(&window.element) as CFTypeRef;
    if minimized {
        set_bool_attr(el, "AXMinimized", false);
    }
    let raise = NSString::from_str("AXRaise");
    // SAFETY: element and action name are live CF objects for the whole call.
    unsafe { AXUIElementPerformAction(el, cfstr(&raise)) };
    set_bool_attr(el, "AXMain", true);
    if !make_frontmost(pid) {
        super::workspace::open_running(&app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_without_windows_has_no_focused_window_title() {
        assert_eq!(focused_window_title(std::process::id() as i32), None);
        assert_eq!(focused_window_title(-1), None);
    }
}
