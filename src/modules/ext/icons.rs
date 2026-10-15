//! Icon files in a reply. `proto` checks only the path's form (absolute, an image
//! extension); this checks, on the worker, that the file is there and starts like an image.
//! An icon that fails is dropped with a warning, and the item keeps the module's icon.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use super::proto::item::ExtItem;
use super::proto::{Parsed, Reply};
use crate::core::Icon;

/// `parsed` without the icon files that are missing or are not images.
pub fn check(mut parsed: Parsed) -> Parsed {
    let items = match &mut parsed.reply {
        Reply::Items(items) => items,
        Reply::View(view) => &mut view.items,
        Reply::Form(_) | Reply::Done(_) | Reply::Ack => return parsed,
    };
    for item in items.iter_mut() {
        check_item(item, &mut parsed.warnings);
    }
    parsed
}

fn check_item(item: &mut ExtItem, w: &mut Vec<String>) {
    let at = format!("item {:?}", item.key);
    drop_bad(&mut item.icon, &at, w);
    for a in &mut item.actions {
        drop_bad(&mut a.icon, &format!("{at} action {:?}", a.key), w);
    }
}

fn drop_bad(icon: &mut Option<Icon>, at: &str, w: &mut Vec<String>) {
    let Some(Icon::File(path)) = icon else { return };
    if !is_image(path) {
        w.push(format!("{at}: icon file {} is missing or not an image; ignored", path.display()));
        *icon = None;
    }
}

/// The file at `path` exists and its first bytes are an image format's.
fn is_image(path: &Path) -> bool {
    let mut head = Vec::with_capacity(12);
    let read = File::open(path).and_then(|f| f.take(12).read_to_end(&mut head));
    read.is_ok() && sniff(&head)
}

/// `head` starts like PNG, JPEG, GIF, TIFF, ICNS, PDF, WebP or HEIC (an ISO `ftyp` box).
fn sniff(head: &[u8]) -> bool {
    const MAGIC: &[&[u8]] =
        &[b"\x89PNG", b"\xff\xd8\xff", b"GIF8", b"II*\0", b"MM\0*", b"icns", b"%PDF"];
    let webp = head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP");
    let ftyp = head.get(4..8) == Some(b"ftyp");
    MAGIC.iter().any(|m| head.starts_with(m)) || webp || ftyp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::scratch::Scratch;
    use crate::modules::ext::proto::{self, Op};

    #[test]
    fn image_formats_are_known_by_their_first_bytes() {
        for head in [&b"\x89PNG\r\n"[..], b"\xff\xd8\xff\xe0", b"GIF89a", b"II*\0", b"MM\0*"] {
            assert!(sniff(head), "{head:?}");
        }
        assert!(sniff(b"icns") && sniff(b"%PDF-1.7") && sniff(b"RIFF\0\0\0\0WEBP"));
        assert!(sniff(b"\0\0\0\x18ftypheic"));
        assert!(!sniff(b"") && !sniff(b"RIFF\0\0\0\0WAVE") && !sniff(b"#!/bin/sh"));
    }

    #[test]
    fn missing_and_non_image_files_are_dropped_with_a_warning() {
        let dir = Scratch::new("ext-icons");
        let (png, text) = (dir.join("a.png"), dir.join("b.png"));
        std::fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(&text, b"hello").unwrap();
        let file = |p: &Path| format!(r#"{{"file": "{}"}}"#, p.display());
        let missing = dir.join("c.png");
        let out = format!(
            r#"{{"items": [{{"key": "a", "title": "A", "icon": {}, "actions": [
                {{"key": "x", "title": "X", "icon": {}}}, {{"key": "y", "title": "Y", "icon": "star"}}]}},
              {{"key": "b", "title": "B", "icon": {}}}]}}"#,
            file(&png),
            file(&missing),
            file(&text)
        );
        let parsed = check(proto::parse(Op::Root, out.as_bytes()).unwrap());
        let Reply::Items(items) = &parsed.reply else { panic!("{:?}", parsed.reply) };
        assert_eq!(items[0].icon, Some(Icon::File(png.clone())));
        assert_eq!((items[0].actions[0].icon.as_ref(), items[1].icon.as_ref()), (None, None));
        assert_eq!(items[0].actions[1].icon, Some(Icon::symbol("star")));
        let w = &parsed.warnings;
        assert_eq!(w.len(), 2, "{w:?}");
        assert_eq!(w[0], format!("item \"a\" action \"x\": icon file {} is missing or not an image; ignored", missing.display()));
        assert!(w[1].starts_with("item \"b\": icon file "));
        let view = format!(r#"{{"items": [{{"key": "b", "title": "B", "icon": {}}}]}}"#, file(&text));
        let parsed = check(proto::parse(Op::Open, view.as_bytes()).unwrap());
        let Reply::View(v) = &parsed.reply else { panic!("{:?}", parsed.reply) };
        assert_eq!((v.items[0].icon.as_ref(), parsed.warnings.len()), (None, 1));
        let done = proto::parse(Op::Act, b"{}").unwrap();
        assert_eq!(check(done.clone()), done);
    }
}
