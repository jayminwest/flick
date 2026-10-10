//! The general pasteboard: plain text, PNG images, and a snapshot of everything on it so a
//! transient write (dictation's paste) can be undone. Text copied out of a private surface
//! goes through `set_text_concealed`, which clip history and clipboard managers skip.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSBitmapImageRep, NSPasteboard, NSPasteboardItem, NSPasteboardTypePNG, NSPasteboardTypeString,
    NSPasteboardTypeTIFF, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSString};

/// nspasteboard.org markers: clipboard managers (and `copied_text`) skip content that has
/// either type.
const TRANSIENT: &str = "org.nspasteboard.TransientType";
const CONCEALED: &str = "org.nspasteboard.ConcealedType";

/// Bumps whenever any app writes the pasteboard; there is no change notification to observe.
pub fn change_count() -> isize {
    NSPasteboard::generalPasteboard().changeCount()
}

/// The pasteboard's text, unless password managers mark it as concealed or transient.
pub fn copied_text() -> Option<String> {
    copied_text_from(&NSPasteboard::generalPasteboard())
}

fn copied_text_from(pb: &NSPasteboard) -> Option<String> {
    let skip = pb.types().is_some_and(|types| {
        types.iter().any(|t| {
            let t = t.to_string();
            t == CONCEALED || t == TRANSIENT
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

/// Replace the pasteboard's contents with `text` marked `org.nspasteboard.ConcealedType` and
/// `TransientType`, so Flick's clip history (`copied_text`) and other clipboard managers do
/// not record it. For copies out of a private surface.
pub fn set_text_concealed(text: &str) {
    set_transient_text_on(&NSPasteboard::generalPasteboard(), text);
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

/// Every item on the pasteboard with the data of each of its types, for `restore`.
/// Types whose data the owner only promised and cannot give now are left out.
pub struct Snapshot {
    items: Vec<Vec<(Retained<NSString>, Retained<NSData>)>>,
}

impl Snapshot {
    /// Whether the pasteboard held nothing readable.
    #[cfg_attr(not(test), expect(dead_code, reason = "dictation insertion (flick-a085) logs it"))]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Copy every item and type of the general pasteboard.
pub fn snapshot() -> Snapshot {
    snapshot_of(&NSPasteboard::generalPasteboard())
}

/// Put `snap` back on the general pasteboard; returns the new change count.
pub fn restore(snap: &Snapshot) -> isize {
    restore_to(&NSPasteboard::generalPasteboard(), snap)
}

/// Replace the general pasteboard's contents with `text` marked transient and concealed,
/// so clip history and clipboard managers skip it; returns the new change count (restore
/// only while it is still this).
pub fn set_transient_text(text: &str) -> isize {
    set_transient_text_on(&NSPasteboard::generalPasteboard(), text)
}

fn snapshot_of(pb: &NSPasteboard) -> Snapshot {
    let items = pb.pasteboardItems().map(|items| items.to_vec()).unwrap_or_default();
    let items = items
        .iter()
        .map(|item| {
            item.types()
                .iter()
                .filter_map(|t| item.dataForType(&t).map(|data| (t, data)))
                .collect::<Vec<_>>()
        })
        .filter(|types| !types.is_empty())
        .collect();
    Snapshot { items }
}

fn restore_to(pb: &NSPasteboard, snap: &Snapshot) -> isize {
    pb.clearContents();
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = snap
        .items
        .iter()
        .map(|types| {
            let item = NSPasteboardItem::new();
            for (t, data) in types {
                item.setData_forType(data, t);
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    if !items.is_empty() {
        pb.writeObjects(&NSArray::from_retained_slice(&items));
    }
    pb.changeCount()
}

fn set_transient_text_on(pb: &NSPasteboard, text: &str) -> isize {
    pb.clearContents();
    // SAFETY: NSPasteboardTypeString is an immutable framework constant, set at load time.
    let string_type = unsafe { NSPasteboardTypeString };
    pb.setString_forType(&NSString::from_str(text), string_type);
    let empty = NSData::new();
    for marker in [TRANSIENT, CONCEALED] {
        pb.setData_forType(Some(&empty), &NSString::from_str(marker));
    }
    pb.changeCount()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private pasteboard, so tests never touch the user's clipboard.
    struct Private(Retained<NSPasteboard>);

    impl Private {
        fn new() -> Private {
            Private(NSPasteboard::pasteboardWithUniqueName())
        }
    }

    impl Drop for Private {
        fn drop(&mut self) {
            // SAFETY: `releaseGlobally` takes no arguments and returns nothing; the unique
            // pasteboard is ours and used no more.
            let () = unsafe { objc2::msg_send![&*self.0, releaseGlobally] };
        }
    }

    fn types(pb: &NSPasteboard) -> Vec<String> {
        pb.types().map(|t| t.iter().map(|t| t.to_string()).collect()).unwrap_or_default()
    }

    fn data(pb: &NSPasteboard, t: &str) -> Option<Vec<u8>> {
        pb.dataForType(&NSString::from_str(t)).map(|d| d.to_vec())
    }

    #[test]
    fn transient_text_is_marked_and_bumps_the_change_count() {
        let pb = Private::new();
        let before = pb.0.changeCount();
        let after = set_transient_text_on(&pb.0, "hello there");
        assert!(after > before);
        assert_eq!(after, pb.0.changeCount());
        let t = types(&pb.0);
        assert!(t.iter().any(|t| t == TRANSIENT), "{t:?}");
        assert!(t.iter().any(|t| t == CONCEALED), "{t:?}");
        // SAFETY: NSPasteboardTypeString is an immutable framework constant.
        let string_type = unsafe { NSPasteboardTypeString };
        let text = pb.0.stringForType(string_type).map(|s| s.to_string());
        assert_eq!(text.as_deref(), Some("hello there"));
    }

    #[test]
    fn concealed_text_is_pasteable_but_clip_history_skips_it() {
        let pb = Private::new();
        // SAFETY: NSPasteboardTypeString is an immutable framework constant.
        let string_type = unsafe { NSPasteboardTypeString };
        pb.0.clearContents();
        pb.0.setString_forType(&NSString::from_str("plain"), string_type);
        assert_eq!(copied_text_from(&pb.0).as_deref(), Some("plain"));
        // `set_text_concealed` is this writer on the general pasteboard.
        set_transient_text_on(&pb.0, "copied privately");
        let t = types(&pb.0);
        assert!(t.iter().any(|t| t == CONCEALED) && t.iter().any(|t| t == TRANSIENT), "{t:?}");
        let pasted = pb.0.stringForType(string_type).map(|s| s.to_string());
        assert_eq!(pasted.as_deref(), Some("copied privately"));
        assert_eq!(copied_text_from(&pb.0), None);
        // Not called: it would replace the user's clipboard.
        let _: fn(&str) = set_text_concealed;
    }

    #[test]
    fn snapshot_and_restore_round_trip_every_item_and_type() {
        let pb = Private::new();
        // The pasteboard server on CI runners sometimes reports a fresh write late; retry.
        let mut snap = snapshot_of(&pb.0);
        for _ in 0..5 {
            let first = NSPasteboardItem::new();
            first.setData_forType(
                &NSData::with_bytes(b"plain"),
                &NSString::from_str("public.utf8-plain-text"),
            );
            first.setData_forType(
                &NSData::with_bytes(&[1, 2, 3]),
                &NSString::from_str("com.example.flick-test"),
            );
            let second = NSPasteboardItem::new();
            second.setData_forType(
                &NSData::with_bytes(b"two"),
                &NSString::from_str("com.example.flick-second"),
            );
            let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
                vec![ProtocolObject::from_retained(first), ProtocolObject::from_retained(second)];
            pb.0.clearContents();
            assert!(pb.0.writeObjects(&NSArray::from_retained_slice(&items)));
            snap = snapshot_of(&pb.0);
            if snap.items.len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(!snap.is_empty());
        assert_eq!(snap.items.len(), 2);

        let written = set_transient_text_on(&pb.0, "dictated");
        assert!(data(&pb.0, "com.example.flick-test").is_none());
        let restored = restore_to(&pb.0, &snap);
        assert!(restored > written);
        assert_eq!(data(&pb.0, "public.utf8-plain-text").as_deref(), Some(&b"plain"[..]));
        assert_eq!(data(&pb.0, "com.example.flick-test").as_deref(), Some(&[1u8, 2, 3][..]));
        let back = pb.0.pasteboardItems().map(|i| i.len());
        assert_eq!(back, Some(2));
        assert!(!types(&pb.0).iter().any(|t| t == TRANSIENT));
    }

    #[test]
    fn an_empty_snapshot_restores_an_empty_pasteboard() {
        let pb = Private::new();
        pb.0.clearContents();
        let snap = snapshot_of(&pb.0);
        assert!(snap.is_empty());
        set_transient_text_on(&pb.0, "x");
        restore_to(&pb.0, &snap);
        assert!(types(&pb.0).is_empty());
        // Not called: they would replace the user's clipboard.
        let _: fn() -> Snapshot = snapshot;
        let _: fn(&Snapshot) -> isize = restore;
        let _: fn(&str) -> isize = set_transient_text;
    }
}
