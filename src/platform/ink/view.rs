//! `FlickInkView`: an `NSView` that holds a `Canvas`, feeds it mouse and key events and
//! draws it. The view is flipped (origin top-left), so its coordinates are the model's.
//!
//! Tool and color keys, undo, redo and Delete act on the view itself; Return, cmd+C, cmd+S
//! and Esc go to the `on_command` callback, which runs while `AppKit` is mid-event (post an
//! event rather than borrow app state, mx-fcbc43). The Text tool opens an `NSTextField` at
//! the click: Return places the text, Esc drops it, a click elsewhere places it.
//!
//! `draw_shapes` paints both the view on screen and the flattened export in `flatten`.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSBezierPath, NSBitmapImageFileType, NSBitmapImageRep, NSColor, NSControl,
    NSControlTextEditingDelegate, NSCursor, NSDeviceRGBColorSpace, NSEvent, NSEventModifierFlags,
    NSFocusRingType, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSImage,
    NSLineCapStyle, NSLineJoinStyle, NSResponder, NSStringDrawing, NSTextField,
    NSTextFieldDelegate, NSTextView, NSView,
};
use objc2_foundation::{
    NSDictionary, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo, NSRect, NSSize, NSString,
};

use super::model::{
    Canvas, Color, Command, Point, Shape, Style, Tool, arrow_head, key_command, normalized, scaled,
};
use crate::platform::Rect;

/// Text at the default 4 pt stroke width is 18 pt.
const TEXT_PT_PER_WIDTH: f64 = 18.0 / 4.0;
/// Highlighter strokes are this much wider than pen strokes, and this opaque.
const HIGHLIGHT_WIDTH: f64 = 3.0;
const HIGHLIGHT_ALPHA: f32 = 0.35;
/// The text field's cell draws its text this far inside the field's frame.
const FIELD_INSET: f64 = 2.0;

/// What a `FlickInkView` holds.
pub struct Ink {
    canvas: RefCell<Canvas>,
    style: RefCell<Style>,
    tool: Cell<Tool>,
    color: Cell<Color>,
    /// Model points to view points: 1 on screen, pixels per point in `flatten`.
    scale: Cell<f64>,
    background: RefCell<Option<Retained<NSImage>>>,
    on_command: Cell<Option<fn(Command)>>,
    /// Runs after a shape is finished (mouse up or placed text).
    on_stroke: Cell<Option<fn()>>,
    /// The open text field and the point its text goes to.
    text: RefCell<Option<(Retained<NSTextField>, Point)>>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickInkView"]
    #[ivars = Ink]
    pub struct FlickInkView;

    // SAFETY: the protocols' methods are optional; the one below has its exact signature.
    unsafe impl NSObjectProtocol for FlickInkView {}
    // SAFETY: as above.
    unsafe impl NSControlTextEditingDelegate for FlickInkView {}
    // SAFETY: as above.
    unsafe impl NSTextFieldDelegate for FlickInkView {}

    impl FlickInkView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        // The first click into an inactive window draws instead of only focusing it.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::crosshairCursor());
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let ink = self.ivars();
            if let Some(image) = &*ink.background.borrow() {
                image.drawInRect(self.bounds());
            }
            draw_shapes(ink.canvas.borrow().visible(), &ink.style.borrow(), ink.scale.get());
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.end_text(true);
            let p = self.point(event);
            let ink = self.ivars();
            if ink.tool.get() == Tool::Text {
                self.open_text(p);
            } else {
                ink.canvas.borrow_mut().begin(ink.tool.get(), ink.color.get(), p, now());
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let p = self.point(event);
            self.ivars().canvas.borrow_mut().drag(p);
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            let added = self.ivars().canvas.borrow_mut().end();
            self.setNeedsDisplay(true);
            self.stroked(added);
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !self.key(event) {
                // SAFETY: the superclass method, with the argument it was called with.
                unsafe { msg_send![super(self), keyDown: event] }
            }
        }

        // Cmd keys reach the window's views before any first responder; with no main menu
        // nothing else claims cmd+Z, cmd+C or cmd+S.
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            event.modifierFlags().contains(NSEventModifierFlags::Command) && self.key(event)
                // SAFETY: the superclass method, with the argument it was called with.
                || unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn text_command(&self, _control: &NSControl, _view: &NSTextView, sel: Sel) -> bool {
            // Unknown selectors fall through to the field editor (a tail expression: an early
            // `return` in a class method trips the macro's Bool conversion).
            let keep = (sel == sel!(insertNewline:))
                .then_some(true)
                .or((sel == sel!(cancelOperation:)).then_some(false));
            keep.inspect(|&keep| self.end_text(keep)).is_some()
        }
    }
);

impl FlickInkView {
    /// An empty canvas of `frame` drawn in `style`, with the arrow tool and the first color.
    pub fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        style: Style,
        on_command: Option<fn(Command)>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ink {
            canvas: RefCell::new(Canvas::new(style.width)),
            style: RefCell::new(style),
            tool: Cell::new(Tool::default()),
            color: Cell::new(Color::default()),
            scale: Cell::new(1.0),
            background: RefCell::new(None),
            on_command: Cell::new(on_command),
            on_stroke: Cell::new(None),
            text: RefCell::new(None),
        });
        // SAFETY: NSView's designated initializer, with the argument type of its signature.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Draw `image` under the shapes, stretched to the bounds.
    pub fn set_background(&self, image: Option<&NSImage>) {
        *self.ivars().background.borrow_mut() = image.map(Retained::from);
        self.setNeedsDisplay(true);
    }

    /// New shapes take `style`'s width and colors; existing shapes are redrawn in its colors.
    pub fn set_style(&self, style: Style) {
        let ink = self.ivars();
        ink.canvas.borrow_mut().width = style.width;
        *ink.style.borrow_mut() = style;
        self.setNeedsDisplay(true);
    }

    /// Call `f` after each finished shape. It runs while `AppKit` is mid-event.
    pub fn set_on_stroke(&self, f: Option<fn()>) {
        self.ivars().on_stroke.set(f);
    }

    /// No finished and no active shape.
    pub fn is_empty(&self) -> bool {
        self.ivars().canvas.borrow().is_empty()
    }

    /// Drop every shape. Returns whether anything was visible.
    pub fn clear(&self) -> bool {
        self.end_text(false);
        let cleared = self.ivars().canvas.borrow_mut().clear();
        self.setNeedsDisplay(true);
        cleared
    }

    /// Drop shapes started `fade_secs` or more before `now` (see `Canvas::expire`).
    pub fn expire(&self, now: f64, fade_secs: f32) -> bool {
        let gone = self.ivars().canvas.borrow_mut().expire(now, fade_secs);
        if gone {
            self.setNeedsDisplay(true);
        }
        gone
    }

    /// The background and shapes at `width` x `height` pixels as PNG bytes, the shapes scaled
    /// from the view's points; `size` (points) sets the PNG's resolution. Draws an offscreen
    /// copy of the view; no window.
    pub fn flatten(&self, width: usize, height: usize, size: NSSize) -> Option<Vec<u8>> {
        let ink = self.ivars();
        let px = NSRect::new(NSPoint::ZERO, NSSize::new(width as f64, height as f64));
        let copy = Self::new(self.mtm(), px, ink.style.borrow().clone(), None);
        *copy.ivars().canvas.borrow_mut() = ink.canvas.borrow().clone();
        copy.ivars().scale.set(export_scale(width, self.bounds().size.width));
        copy.set_background(ink.background.borrow().as_deref());
        let rep = bitmap(width, height)?;
        copy.cacheDisplayInRect_toBitmapImageRep(px, &rep);
        rep.setSize(size);
        // SAFETY: an empty properties dictionary is valid for PNG encoding.
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }?;
        Some(png.to_vec())
    }

    /// The event's location in view points.
    fn point(&self, event: &NSEvent) -> Point {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        Point::new(p.x, p.y)
    }

    /// Handle a key press; false when it maps to nothing or a text field is open.
    fn key(&self, event: &NSEvent) -> bool {
        if self.ivars().text.borrow().is_some() {
            return false;
        }
        let flags = event.modifierFlags();
        let chars = event.charactersIgnoringModifiers().map(|s| s.to_string());
        let command = key_command(
            chars.as_deref().unwrap_or(""),
            flags.contains(NSEventModifierFlags::Command),
            flags.contains(NSEventModifierFlags::Shift),
        );
        command.inspect(|&c| self.apply(c)).is_some()
    }

    fn apply(&self, command: Command) {
        let ink = self.ivars();
        let mut canvas = ink.canvas.borrow_mut();
        match command {
            Command::Tool(tool) => ink.tool.set(tool),
            Command::Color(color) => ink.color.set(color),
            Command::Undo => _ = canvas.undo(),
            Command::Redo => _ = canvas.redo(),
            Command::Clear => _ = canvas.clear(),
            Command::Done | Command::Copy | Command::Save | Command::Cancel => {
                // The callback may flatten this view, which borrows the canvas.
                drop(canvas);
                if let Some(on_command) = ink.on_command.get() {
                    on_command(command);
                }
                return;
            }
        }
        self.setNeedsDisplay(true);
    }

    /// Open a text field with its text's top-left at `p`, in the current color and size.
    fn open_text(&self, p: Point) {
        let ink = self.ivars();
        let size = text_size(ink.canvas.borrow().width);
        let r = field_frame(p, size, self.bounds().size.width);
        let frame = NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h));
        let field = NSTextField::initWithFrame(NSTextField::alloc(self.mtm()), frame);
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setUsesSingleLineMode(true);
        field.setFont(Some(&NSFont::systemFontOfSize(size)));
        field.setTextColor(Some(&ns_color(ink.style.borrow().rgba(ink.color.get()))));
        // SAFETY: the view outlives the field, its subview (the field holds it weakly).
        unsafe { field.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(self))) };
        self.addSubview(&field);
        if let Some(window) = self.window() {
            window.makeFirstResponder(Some(&field));
        }
        *ink.text.borrow_mut() = Some((field, p));
    }

    /// Close the open text field, placing its text when `keep`.
    fn end_text(&self, keep: bool) {
        let ink = self.ivars();
        let Some((field, p)) = ink.text.borrow_mut().take() else { return };
        let text = field.stringValue().to_string();
        let added = keep && ink.canvas.borrow_mut().add_text(ink.color.get(), p, &text, now());
        if let Some(window) = self.window() {
            window.makeFirstResponder(Some(self));
        }
        field.removeFromSuperview();
        self.setNeedsDisplay(true);
        self.stroked(added);
    }

    fn stroked(&self, added: bool) {
        if let Some(on_stroke) = self.ivars().on_stroke.get().filter(|_| added) {
            on_stroke();
        }
    }
}

/// Seconds since boot: the clock shapes are stamped with.
pub fn now() -> f64 {
    NSProcessInfo::processInfo().systemUptime()
}

/// Paint `shapes` into the current (flipped) graphics context, each scaled by `scale`.
pub fn draw_shapes<'a>(shapes: impl Iterator<Item = &'a Shape>, style: &Style, scale: f64) {
    for shape in shapes {
        draw_shape(&scaled(shape, scale), style);
    }
}

fn draw_shape(shape: &Shape, style: &Style) {
    let (rgba, width) = paint(shape, style);
    let color = ns_color(rgba);
    match (shape.tool, shape.points.as_slice()) {
        (Tool::Pen | Tool::Highlight, [first, rest @ ..]) => {
            let path = line_path(width);
            path.moveToPoint(ns_point(*first));
            // A click is a dot: a zero-length segment with round caps.
            for p in if rest.is_empty() { std::slice::from_ref(first) } else { rest } {
                path.lineToPoint(ns_point(*p));
            }
            color.setStroke();
            path.stroke();
        }
        (Tool::Arrow, &[from, to]) => {
            let head = arrow_head(from, to, shape.width);
            if let Some(end) = shaft_end(from, head) {
                let path = line_path(width);
                path.moveToPoint(ns_point(from));
                path.lineToPoint(ns_point(end));
                color.setStroke();
                path.stroke();
            }
            let tip = NSBezierPath::bezierPath();
            tip.moveToPoint(ns_point(head[0]));
            tip.lineToPoint(ns_point(head[1]));
            tip.lineToPoint(ns_point(head[2]));
            tip.closePath();
            color.setFill();
            tip.fill();
        }
        (Tool::Rect, &[a, b]) => {
            let path = NSBezierPath::bezierPathWithRect(ns_rect(normalized(a, b)));
            path.setLineWidth(width);
            color.setStroke();
            path.stroke();
        }
        (Tool::Redact, &[a, b]) => {
            NSColor::blackColor().setFill();
            NSBezierPath::fillRect(ns_rect(normalized(a, b)));
        }
        (Tool::Text, &[p]) => {
            let Some(text) = &shape.text else { return };
            let font = NSFont::systemFontOfSize(text_size(shape.width));
            let font: &AnyObject = font.as_ref();
            let color: &AnyObject = color.as_ref();
            // SAFETY: the attribute keys are immutable framework constants, set at load time.
            let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
            let attrs = NSDictionary::from_slices(&keys, &[font, color]);
            // SAFETY: the font attribute is an NSFont and the color attribute an NSColor.
            unsafe {
                NSString::from_str(text).drawAtPoint_withAttributes(ns_point(p), Some(&attrs));
            };
        }
        // An arrow, box or redact still at its first point, or a malformed shape.
        _ => {}
    }
}

fn line_path(width: f64) -> Retained<NSBezierPath> {
    let path = NSBezierPath::bezierPath();
    path.setLineWidth(width);
    path.setLineCapStyle(NSLineCapStyle::Round);
    path.setLineJoinStyle(NSLineJoinStyle::Round);
    path
}

/// A pixels-wide, pixels-high RGBA bitmap.
fn bitmap(width: usize, height: usize) -> Option<Retained<NSBitmapImageRep>> {
    // SAFETY: the color space name is an immutable framework constant, set at load time.
    let space = unsafe { NSDeviceRGBColorSpace };
    // SAFETY: null planes make the rep allocate its own buffer; 8 bits x 4 samples, meshed,
    // with 0 bytes per row and bits per pixel letting it compute both.
    unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width as isize,
            height as isize,
            8,
            4,
            true,
            false,
            space,
            0,
            0,
        )
    }
}

pub(super) fn ns_color([r, g, b, a]: [f32; 4]) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r.into(), g.into(), b.into(), a.into())
}

fn ns_point(p: Point) -> NSPoint {
    NSPoint::new(p.x, p.y)
}

fn ns_rect(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}

/// The color and line width `shape` is stroked with: highlighter strokes are wide and
/// translucent.
fn paint(shape: &Shape, style: &Style) -> ([f32; 4], f64) {
    let [r, g, b, a] = style.rgba(shape.color);
    let width = f64::from(shape.width);
    if shape.tool == Tool::Highlight {
        ([r, g, b, a * HIGHLIGHT_ALPHA], width * HIGHLIGHT_WIDTH)
    } else {
        ([r, g, b, a], width)
    }
}

/// Where an arrow's line stops: the middle of the head's base, so the line's cap never pokes
/// past the tip. `None` when the arrow is shorter than its head.
fn shaft_end(from: Point, [left, tip, right]: [Point; 3]) -> Option<Point> {
    let base = Point::new(f64::midpoint(left.x, right.x), f64::midpoint(left.y, right.y));
    let along = (base.x - from.x) * (tip.x - from.x) + (base.y - from.y) * (tip.y - from.y);
    (along > 0.0).then_some(base)
}

/// Font size for text drawn at stroke `width`.
fn text_size(width: f32) -> f64 {
    f64::from(width) * TEXT_PT_PER_WIDTH
}

/// The text field for text at `p` in a `size` pt font: its text starts at `p`, and it runs
/// to the view's right edge (`view_w`), at least 80 pt wide.
fn field_frame(p: Point, size: f64, view_w: f64) -> Rect {
    let x = p.x - FIELD_INSET;
    Rect { x, y: p.y - FIELD_INSET, w: (view_w - x).max(80.0), h: (size * 1.4).ceil() }
}

/// Pixels per view point when `view_w` points become `pixels` wide; 1 for an empty view.
fn export_scale(pixels: usize, view_w: f64) -> f64 {
    if view_w > 0.0 { pixels as f64 / view_w } else { 1.0 }
}

#[cfg(test)]
mod tests;
