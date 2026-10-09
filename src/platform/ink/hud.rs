//! Draws the key legend (`legend`) into a flipped `FlickInkView`: a dark rounded strip at
//! the bottom with the palette swatches (the current one ringed), the current tool and
//! color, and the keys. `?` hides and shows it, for every view at once. Also the
//! model-to-`AppKit` conversions the views share.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSStringDrawing,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

use super::legend::{self, Mode};
use super::model::{Color, Point, Style, Tool};
use crate::platform::Rect;

const FONT_PT: f64 = 12.0;
const LINE_GAP: f64 = 3.0;
const CORNER: f64 = 10.0;

thread_local! {
    /// The legend is hidden (`?` toggles it); shared by the overlay's displays and the editor.
    static HIDDEN: Cell<bool> = const { Cell::new(false) };
}

/// Flip whether the legend shows.
pub fn toggle() {
    HIDDEN.set(!HIDDEN.get());
}

/// Paint the legend for `mode` into a flipped view of `bounds`, unless it is hidden.
pub fn draw(bounds: NSRect, mode: Mode, tool: Tool, color: Color, style: &Style) {
    if HIDDEN.get() {
        return;
    }
    let lines = legend::lines(mode, tool, color, style);
    let font = NSFont::systemFontOfSize(FONT_PT);
    let bold = NSFont::boldSystemFontOfSize(FONT_PT);
    let white = NSColor::whiteColor();
    let dim = NSColor::colorWithWhite_alpha(1.0, 0.75);
    let attrs = [attributes(&bold, &white), attributes(&font, &dim), attributes(&font, &dim)];
    let texts: Vec<Retained<NSString>> = lines.iter().map(|l| NSString::from_str(l)).collect();
    let sizes: Vec<NSSize> =
        // SAFETY: each dictionary holds an NSFont and an NSColor under the matching keys.
        texts.iter().zip(&attrs).map(|(t, a)| unsafe { t.sizeWithAttributes(Some(a)) }).collect();
    let text_w = sizes.iter().map(|s| s.width).fold(0.0, f64::max);
    let line_h = sizes.iter().map(|s| s.height).fold(0.0, f64::max);
    let text_h = 3.0 * line_h + 2.0 * LINE_GAP;
    let n = style.palette.len();
    let strip = legend::frame(bounds.size.width, bounds.size.height, text_w, text_h, n);

    let back =
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(ns_rect(strip), CORNER, CORNER);
    NSColor::colorWithWhite_alpha(0.08, 0.82).setFill();
    back.fill();

    for (i, r) in legend::swatches(strip, n).into_iter().enumerate() {
        let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(ns_rect(r), 3.0, 3.0);
        ns_color(style.rgba(Color(u8::try_from(i).unwrap_or(u8::MAX)))).setFill();
        path.fill();
        let current = usize::from(color.0) == i;
        path.setLineWidth(if current { 2.5 } else { 0.5 });
        (if current { &white } else { &dim }).setStroke();
        path.stroke();
    }

    let mut y = strip.y + legend::PAD + legend::SWATCH + legend::GAP;
    for (text, a) in texts.iter().zip(&attrs) {
        let at = NSPoint::new(strip.x + legend::PAD, y);
        // SAFETY: as above.
        unsafe { text.drawAtPoint_withAttributes(at, Some(a)) };
        y += line_h + LINE_GAP;
    }
}

fn attributes(font: &NSFont, color: &NSColor) -> Retained<NSDictionary<NSString, AnyObject>> {
    let font: &AnyObject = font.as_ref();
    let color: &AnyObject = color.as_ref();
    // SAFETY: the attribute keys are immutable framework constants, set at load time.
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    NSDictionary::from_slices(&keys, &[font, color])
}

pub(super) fn ns_color([r, g, b, a]: [f32; 4]) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r.into(), g.into(), b.into(), a.into())
}

pub(super) fn ns_point(p: Point) -> NSPoint {
    NSPoint::new(p.x, p.y)
}

pub(super) fn ns_rect(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}
