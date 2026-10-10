//! A surface's input: a plain-text `NSTextView` in a borderless scroll view on a rounded
//! fill, with a placeholder label that shows while it is empty. One line or about four
//! (`Input`); Return and the other keys are routed by the shared delegate in `mod.rs`.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSBorderType, NSBox, NSBoxType, NSColor, NSFont, NSLineBreakMode, NSResponder, NSScrollView,
    NSTextField, NSTextView, NSTitlePosition, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::geometry::Rect;
use super::window::Delegate;

const FONT: f64 = 14.0;
/// Text inset inside the box, and where the placeholder sits to match it.
const INSET: NSSize = NSSize::new(6.0, 4.0);
const PLACEHOLDER_X: f64 = 11.0;

pub(super) fn ns_rect(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}

/// A filled, borderless box with rounded corners.
pub(super) fn filled(mtm: MainThreadMarker, color: &NSColor, radius: f64) -> Retained<NSBox> {
    // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
    let b: Retained<NSBox> = unsafe { msg_send![NSBox::alloc(mtm), initWithFrame: NSRect::ZERO] };
    b.setBoxType(NSBoxType::Custom);
    b.setTitlePosition(NSTitlePosition::NoTitle);
    b.setBorderWidth(0.0);
    b.setCornerRadius(radius);
    b.setFillColor(color);
    b
}

pub(super) struct Field {
    back: Retained<NSBox>,
    scroll: Retained<NSScrollView>,
    text: Retained<NSTextView>,
    placeholder: Retained<NSTextField>,
}

impl Field {
    /// The input, added to `root` (flipped), reporting to `delegate`.
    pub(super) fn new(
        mtm: MainThreadMarker,
        root: &NSView,
        delegate: &Delegate,
        placeholder: &str,
        multi: bool,
    ) -> Field {
        let back = filled(mtm, &NSColor::quaternaryLabelColor(), 8.0);
        let scroll = NSTextView::scrollableTextView(mtm);
        scroll.setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setHasVerticalScroller(multi);
        scroll.setAutohidesScrollers(true);
        // `scrollableTextView` makes an NSTextView its document view; the fallback never runs.
        let text = scroll.documentView().and_then(|v| v.downcast::<NSTextView>().ok());
        let text = text.unwrap_or_else(|| {
            let t = NSTextView::initWithFrame(NSTextView::alloc(mtm), NSRect::ZERO);
            scroll.setDocumentView(Some(&t));
            t
        });
        text.setRichText(false);
        text.setImportsGraphics(false);
        text.setAllowsUndo(true);
        text.setDrawsBackground(false);
        text.setFont(Some(&NSFont::systemFontOfSize(FONT)));
        text.setTextColor(Some(&NSColor::labelColor()));
        text.setTextContainerInset(INSET);
        text.setAutomaticQuoteSubstitutionEnabled(false);
        text.setAutomaticDashSubstitutionEnabled(false);
        text.setAutomaticTextReplacementEnabled(false);
        // The text view holds its delegate weakly; the surface module keeps it for the process.
        text.setDelegate(Some(ProtocolObject::from_ref(delegate)));

        let label = NSTextField::labelWithString(&NSString::from_str(placeholder), mtm);
        label.setFont(Some(&NSFont::systemFontOfSize(FONT)));
        label.setTextColor(Some(&NSColor::placeholderTextColor()));
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        root.addSubview(&back);
        root.addSubview(&scroll);
        root.addSubview(&label);
        Field { back, scroll, text, placeholder: label }
    }

    /// Put the input at `r` (top-down in the content view).
    pub(super) fn set_frame(&self, r: Rect) {
        self.back.setFrame(ns_rect(r));
        self.scroll.setFrame(ns_rect(Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0)));
        let line = FONT + 4.0;
        let w = (r.w - PLACEHOLDER_X - 8.0).max(0.0);
        self.placeholder.setFrame(ns_rect(Rect::new(r.x + PLACEHOLDER_X, r.y + 4.0, w, line)));
    }

    pub(super) fn text(&self) -> String {
        self.text.string().to_string()
    }

    /// Replace the text (caret at the end) and show the placeholder if it is now empty.
    pub(super) fn set_text(&self, s: &str) {
        if self.text() != s {
            self.text.setString(&NSString::from_str(s));
        }
        self.sync();
    }

    /// Show the placeholder only while the input is empty.
    pub(super) fn sync(&self) {
        self.placeholder.setHidden(self.text.string().length() > 0);
    }

    pub(super) fn responder(&self) -> &NSResponder {
        &self.text
    }

    pub(super) fn is(&self, view: &NSTextView) -> bool {
        std::ptr::eq(&*self.text, view)
    }
}
