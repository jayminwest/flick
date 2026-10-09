//! The general pasteboard: plain text, and PNG images.

use objc2_app_kit::{
    NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeString,
    NSPasteboardTypeTIFF,
};
use objc2_foundation::{NSData, NSString};

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

/// Replace the pasteboard's contents with a PNG image (`bytes` is the whole file). Also
/// writes TIFF, for apps that read only TIFF; skipped when `NSBitmapImageRep` cannot decode
/// `bytes`.
pub fn set_png(bytes: &[u8]) {
    let png = NSData::with_bytes(bytes);
    let tiff = NSBitmapImageRep::imageRepWithData(&png).and_then(|rep| rep.TIFFRepresentation());
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    // SAFETY: the pasteboard type names are immutable framework constants, set at load time.
    let (png_type, tiff_type) = unsafe { (NSPasteboardTypePNG, NSPasteboardTypeTIFF) };
    pb.setData_forType(Some(&png), png_type);
    if let Some(tiff) = tiff {
        pb.setData_forType(Some(&tiff), tiff_type);
    }
}
