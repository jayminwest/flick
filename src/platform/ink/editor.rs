//! The annotation editor: a titled window showing an image with a `FlickInkView` on top.
//! Return (or cmd+C) flattens the shapes into the image at full pixel size and writes a PNG;
//! Esc or the close button writes nothing. One editor at a time.
//!
//! Flick is an accessory app, so the editor activates it to get keys, and gives activation
//! back to the app that was in front when it closes.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSBackingStoreType, NSBeep, NSImage,
    NSRunningApplication, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use super::model::{Command, Style};
use super::view::FlickInkView;
use crate::platform::{Rect, pasteboard, screens, timer};

/// Room left around a fitted editor for its title bar and the screen edges.
const MARGIN: f64 = 40.0;

/// How the editor draws and what Return does.
pub struct EditOpts {
    pub style: Style,
    /// Return copies the image as well as saving it.
    pub copy: bool,
}

/// How an editor closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    /// The PNG is at `dest`.
    Saved,
    /// The PNG is at `dest` and on the pasteboard.
    Copied,
    /// Nothing was written.
    Cancelled,
}

/// What a command does to the open editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Finish {
    Write { copy: bool },
    Cancel,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickInkEditorDelegate"]
    struct Delegate;

    // SAFETY: the protocols' methods are optional; the one below has its exact signature.
    unsafe impl NSObjectProtocol for Delegate {}
    // SAFETY: as above.
    unsafe impl NSWindowDelegate for Delegate {}

    impl Delegate {
        // The close button. A close from `finish` finds no editor left and does nothing.
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, _n: &NSNotification) {
            if let Some(editor) = EDITOR.with_borrow_mut(Option::take) {
                finish(editor, Done::Cancelled, false);
            }
        }
    }
);

struct Editor {
    window: Retained<NSWindow>,
    view: Retained<FlickInkView>,
    _delegate: Retained<Delegate>,
    dest: PathBuf,
    copy: bool,
    /// The image's size in pixels, and in points (its resolution).
    pixels: (usize, usize),
    points: NSSize,
    on_done: fn(Done),
    /// The app in front when the editor opened.
    previous: Option<Retained<NSRunningApplication>>,
}

thread_local! {
    static EDITOR: RefCell<Option<Editor>> = const { RefCell::new(None) };
}

/// Open the editor on the image at `src`; saving writes a PNG to `dest` (which may be
/// `src`). `on_done` runs once when the editor closes, on the main thread while `AppKit` is
/// mid-event: it must only post an event (mx-fcbc43). False when `src` is not an image, or
/// when an editor is already open (that one comes to the front instead).
pub fn edit(src: &Path, dest: &Path, opts: EditOpts, on_done: fn(Done)) -> bool {
    let mtm = crate::platform::mtm();
    if let Some(window) = EDITOR.with_borrow(|e| e.as_ref().map(|e| e.window.clone())) {
        NSApplication::sharedApplication(mtm).activate();
        window.makeKeyAndOrderFront(None);
        return false;
    }
    let path = NSString::from_str(&src.display().to_string());
    let Some(image) = NSImage::initWithContentsOfFile(NSImage::alloc(), &path) else {
        return false;
    };
    let points = image.size();
    let Some(rep) = image.representations().firstObject() else { return false };
    let (Ok(w), Ok(h)) = (usize::try_from(rep.pixelsWide()), usize::try_from(rep.pixelsHigh()))
    else {
        return false;
    };
    if w == 0 || h == 0 || points.width <= 0.0 || points.height <= 0.0 {
        return false;
    }
    let area = screens::visible_areas().first().copied();
    let (fw, fh) = fit((points.width, points.height), area);
    let frame = NSRect::new(NSPoint::ZERO, NSSize::new(fw, fh));

    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
    // SAFETY: NSWindow's designated initializer, with argument types matching its signature.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: `Editor` holds the window until after it closes, so close never frees it early.
    unsafe { window.setReleasedWhenClosed(false) };
    let name = src.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    window.setTitle(&NSString::from_str(&format!("Annotate {name}")));
    // SAFETY: `init` is NSObject's designated initializer and returns a +1 object.
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    let view = FlickInkView::new(mtm, frame, opts.style, Some(command));
    view.set_background(Some(&image));
    window.setContentView(Some(&view));
    window.center();
    window.makeFirstResponder(Some(&view));

    let previous = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .filter(|app| app.processIdentifier() != std::process::id() as i32);
    NSApplication::sharedApplication(mtm).activate();
    window.makeKeyAndOrderFront(None);
    EDITOR.set(Some(Editor {
        window,
        view,
        _delegate: delegate,
        dest: dest.to_path_buf(),
        copy: opts.copy,
        pixels: (w, h),
        points,
        on_done,
        previous,
    }));
    true
}

/// The view's Return, cmd+C, cmd+S and Esc.
fn command(cmd: Command) {
    let Some(action) = EDITOR.with_borrow(|e| e.as_ref().and_then(|e| finish_for(cmd, e.copy)))
    else {
        return;
    };
    let done = match action {
        Finish::Cancel => Done::Cancelled,
        Finish::Write { copy } => {
            if !EDITOR.with_borrow(|e| e.as_ref().is_some_and(|e| write(e, copy))) {
                NSBeep();
                return;
            }
            if copy { Done::Copied } else { Done::Saved }
        }
    };
    if let Some(editor) = EDITOR.with_borrow_mut(Option::take) {
        finish(editor, done, true);
    }
}

/// What `cmd` does when Return is set to copy (`copy`); `None` for commands the view keeps.
fn finish_for(cmd: Command, copy: bool) -> Option<Finish> {
    match cmd {
        Command::Done => Some(Finish::Write { copy }),
        Command::Copy => Some(Finish::Write { copy: true }),
        Command::Save => Some(Finish::Write { copy: false }),
        Command::Cancel => Some(Finish::Cancel),
        Command::Tool(_) | Command::Color(_) | Command::Undo | Command::Redo | Command::Clear => {
            None
        }
    }
}

/// Flatten the editor's image and shapes to `dest`, and to the pasteboard when `copy`.
fn write(editor: &Editor, copy: bool) -> bool {
    let (w, h) = editor.pixels;
    let Some(png) = editor.view.flatten(w, h, editor.points) else { return false };
    if std::fs::write(&editor.dest, &png).is_err() {
        return false;
    }
    if copy {
        pasteboard::set_png(&png);
    }
    true
}

/// Close `editor` (unless the window is already closing), give activation back and report
/// `done`. The window and view are released on the next run loop pass: this runs inside
/// one of the view's own event methods.
fn finish(editor: Editor, done: Done, close: bool) {
    if close {
        editor.window.close();
    }
    if let Some(app) = &editor.previous {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
    let on_done = editor.on_done;
    let slot = Cell::new(Some(editor));
    timer::after(0.0, move || drop(slot.take()));
    on_done(done);
}

/// The content size for an image of `size` points: the image's own size, shrunk (keeping its
/// aspect ratio) to fit `area` less a margin. Never enlarged.
fn fit((w, h): (f64, f64), area: Option<Rect>) -> (f64, f64) {
    let Some(area) = area else { return (w, h) };
    let room = ((area.w - MARGIN) / w).min((area.h - MARGIN) / h);
    let k = room.clamp(f64::MIN_POSITIVE, 1.0);
    ((w * k).round(), (h * k).round())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::ink::model::{Color, Tool};

    #[test]
    fn return_copies_only_when_configured_and_cmd_keys_override() {
        assert_eq!(finish_for(Command::Done, true), Some(Finish::Write { copy: true }));
        assert_eq!(finish_for(Command::Done, false), Some(Finish::Write { copy: false }));
        for copy in [true, false] {
            assert_eq!(finish_for(Command::Copy, copy), Some(Finish::Write { copy: true }));
            assert_eq!(finish_for(Command::Save, copy), Some(Finish::Write { copy: false }));
            assert_eq!(finish_for(Command::Cancel, copy), Some(Finish::Cancel));
        }
        for cmd in [
            Command::Tool(Tool::Pen),
            Command::Color(Color(2)),
            Command::Undo,
            Command::Redo,
            Command::Clear,
        ] {
            assert_eq!(finish_for(cmd, true), None);
        }
    }

    #[test]
    fn the_editor_fits_the_screen_without_growing() {
        let area = |w, h| Some(Rect { x: 0.0, y: 25.0, w, h });
        assert_eq!(fit((800.0, 600.0), area(1440.0, 875.0)), (800.0, 600.0));
        // A full-screen Retina shot (1440x900 pt) on a 1440x875 area: shrunk, aspect kept.
        assert_eq!(fit((1440.0, 900.0), area(1440.0, 875.0)), (1336.0, 835.0));
        // Wide images are limited by width.
        assert_eq!(fit((4000.0, 1000.0), area(1040.0, 875.0)), (1000.0, 250.0));
        assert_eq!(fit((300.0, 200.0), None), (300.0, 200.0));
    }

    #[test]
    fn edit_options_carry_style_and_copy() {
        let opts = EditOpts { style: Style::default(), copy: true };
        assert!(opts.copy);
        assert_eq!(opts.style.palette.len(), 5);
        assert_eq!(Done::Copied, Done::Copied);
        assert_ne!(Done::Saved, Done::Cancelled);
    }
}
