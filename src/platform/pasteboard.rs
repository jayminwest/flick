//! The general pasteboard, as plain text.

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

/// Bumps whenever any app writes the pasteboard; there is no change notification to observe.
pub fn change_count() -> isize {
    NSPasteboard::generalPasteboard().changeCount()
}

/// The pasteboard's text, unless password managers mark it as concealed or transient.
pub fn copied_text() -> Option<String> {
    let pb = NSPasteboard::generalPasteboard();
    let skip = pb.types().is_some_and(|types| {
        types.iter().any(|t| {
            let t = t.to_string();
            t == "org.nspasteboard.ConcealedType" || t == "org.nspasteboard.TransientType"
        })
    });
    if skip {
        return None;
    }
    // SAFETY: NSPasteboardTypeString is an immutable framework constant, set at load time.
    let string_type = unsafe { NSPasteboardTypeString };
    pb.stringForType(string_type).map(|text| text.to_string())
}

/// Replace the pasteboard's contents with `text`.
pub fn set_text(text: &str) {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    // SAFETY: NSPasteboardTypeString is an immutable framework constant, set at load time.
    let string_type = unsafe { NSPasteboardTypeString };
    pb.setString_forType(&NSString::from_str(text), string_type);
}
