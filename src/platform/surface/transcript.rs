//! The `AppKit` side of a surface's rows area: a borderless scroll view over a flipped
//! document of stacked row views. A bubble is a fill with a selectable, non-editable text view
//! of styled markdown-lite (`render`; `http(s)` links open on click) under a header line with
//! a state badge; a card row is the HUD's card renderer (`hud::embed`) on a fill; a divider
//! is a centred caption. Which views survive an update, the frames, the pin to the bottom and
//! the coalescing are decided in the pure `rows`.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBorderType, NSBox, NSColor, NSControlSize, NSFont, NSFontAttributeName, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSImage, NSImageScaling, NSImageView, NSLineBreakMode,
    NSProgressIndicator, NSProgressIndicatorStyle, NSResponder, NSScrollView, NSTextAlignment,
    NSTextField, NSTextView, NSView,
};
use objc2_foundation::{
    NSAttributedString, NSDictionary, NSMutableAttributedString, NSObject, NSPoint, NSRect, NSSize,
    NSString,
};

use super::geometry::{Rect, Size};
use super::input::{filled, ns_rect};
use super::render;
use super::rows::{
    self, BubbleState, Coalesce, DIVIDER_H, Frames, Owned, Plan, Shape, Side, badge, full_width,
    text_max,
};
use crate::platform::hud::embed::Embedded;

define_class!(
    // A plain view laid out top-down: the document and each row.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickSurfaceFlipped"]
    pub(super) struct Flipped;

    impl Flipped {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

fn flipped(mtm: MainThreadMarker) -> Retained<Flipped> {
    // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
    unsafe { msg_send![Flipped::alloc(mtm), initWithFrame: NSRect::ZERO] }
}

/// Text in one font and color.
fn text(s: &str, font: &NSFont, color: &NSColor) -> Retained<NSAttributedString> {
    let (font, color): (&AnyObject, &AnyObject) = (font.as_ref(), color.as_ref());
    // SAFETY: the attribute keys are immutable framework constants, set at load time.
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let attrs = NSDictionary::from_slices(&keys, &[font, color]);
    // SAFETY: the font attribute is an NSFont and the color attribute an NSColor.
    unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(s),
            Some(&attrs),
        )
    }
}

fn label(mtm: MainThreadMarker) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

struct Bubble {
    side: Side,
    back: Retained<NSBox>,
    view: Retained<NSTextView>,
    string: Retained<NSAttributedString>,
    meta: Retained<NSTextField>,
    meta_w: f64,
    badge: Option<Retained<NSView>>,
    /// The text's size at the last width it was measured at.
    measured: Cell<Option<(f64, Size)>>,
}

enum Parts {
    Bubble(Bubble),
    /// Drawn at layout time, when the width is known.
    Card {
        back: Retained<NSBox>,
        card: Option<Box<Embedded>>,
    },
    Divider(Retained<NSTextField>),
}

/// One row's views.
struct Built {
    row: Owned,
    view: Retained<Flipped>,
    parts: Parts,
}

/// The header line: the header in semibold, two spaces, the time in a lighter color.
fn meta(mtm: MainThreadMarker, header: &str, time: &str) -> (Retained<NSTextField>, f64) {
    let l = label(mtm);
    let small = NSFont::systemFontOfSize(11.0);
    // SAFETY: NSFontWeightSemibold is an immutable framework constant, set at load time.
    let bold = NSFont::systemFontOfSize_weight(11.0, unsafe { NSFontWeightSemibold });
    let s = NSMutableAttributedString::new();
    s.appendAttributedString(&text(header, &bold, &NSColor::secondaryLabelColor()));
    if !header.is_empty() && !time.is_empty() {
        s.appendAttributedString(&text("  ", &small, &NSColor::secondaryLabelColor()));
    }
    s.appendAttributedString(&text(time, &small, &NSColor::tertiaryLabelColor()));
    l.setAttributedStringValue(&s);
    let w = if header.is_empty() && time.is_empty() { 0.0 } else { l.fittingSize().width.ceil() };
    (l, w)
}

/// A spinner or a red mark for `state`, or nothing.
fn badge_view(mtm: MainThreadMarker, state: BubbleState) -> Option<Retained<NSView>> {
    let b = badge(state);
    if b.spinner {
        let s = NSProgressIndicator::initWithFrame(NSProgressIndicator::alloc(mtm), NSRect::ZERO);
        s.setStyle(NSProgressIndicatorStyle::Spinning);
        s.setControlSize(NSControlSize::Mini);
        // SAFETY: nil is a valid sender.
        unsafe { s.startAnimation(None) };
        return Some(s.into_super());
    }
    b.failed.then(|| {
        let i = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::ZERO);
        i.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        i.setContentTintColor(Some(&NSColor::systemRedColor()));
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("exclamationmark.circle.fill"),
            Some(&NSString::from_str("Failed")),
        );
        i.setImage(image.as_deref());
        i.into_super().into_super()
    })
}

/// What a bubble row shows.
struct Says<'a> {
    side: Side,
    header: &'a str,
    time: &'a str,
    md: &'a str,
    state: BubbleState,
}

fn bubble(mtm: MainThreadMarker, into: &NSView, says: &Says) -> Bubble {
    let Says { side, header, time, md, state } = *says;
    let dim = badge(state).dim || side == Side::System;
    let color = if dim { NSColor::secondaryLabelColor() } else { NSColor::labelColor() };
    let size = if side == Side::System { 12.0 } else { 13.0 };
    let string = render::attributed(md, size, &color);
    let fill = match side {
        Side::Mine => NSColor::controlAccentColor().colorWithAlphaComponent(0.22),
        Side::Theirs => NSColor::quaternaryLabelColor(),
        Side::System => NSColor::clearColor(),
    };
    let back = filled(mtm, &fill, 10.0);
    let view = render::text_view(mtm, &string, true);
    let (meta, meta_w) = meta(mtm, header, time);
    let badge = badge_view(mtm, state);
    for v in [&*back as &NSView, &view, &meta].into_iter().chain(badge.as_deref()) {
        into.addSubview(v);
    }
    let measured = Cell::new(None);
    Bubble { side, back, view, string, meta, meta_w, badge, measured }
}

fn build(mtm: MainThreadMarker, doc: &NSView, row: Owned) -> Built {
    let view = flipped(mtm);
    doc.addSubview(&view);
    let parts = match &row {
        Owned::Bubble { side, header, time, md, state, .. } => {
            let says = Says { side: *side, header, time, md, state: *state };
            Parts::Bubble(bubble(mtm, &view, &says))
        }
        Owned::Card { .. } => {
            let back = filled(mtm, &NSColor::quaternaryLabelColor(), 10.0);
            view.addSubview(&back);
            Parts::Card { back, card: None }
        }
        Owned::Divider { text: t } => {
            let l = label(mtm);
            l.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            l.setTextColor(Some(&NSColor::secondaryLabelColor()));
            l.setAlignment(NSTextAlignment::Center);
            l.setStringValue(&NSString::from_str(t));
            view.addSubview(&l);
            Parts::Divider(l)
        }
    };
    Built { row, view, parts }
}

impl Bubble {
    /// The text's size wrapped at `width`'s bubble width (cached per width).
    fn size(&self, width: f64) -> Size {
        if let Some((w, size)) = self.measured.get()
            && (w - width).abs() < 0.5
        {
            return size;
        }
        let (w, h) = render::measure(&self.string, text_max(width, self.side));
        let size = Size { w, h };
        self.measured.set(Some((width, size)));
        size
    }

    fn place(&self, f: &Frames) {
        self.back.setFrame(ns_rect(f.bubble));
        self.view.setFrame(ns_rect(f.text));
        self.meta.setFrame(ns_rect(f.meta));
        if let Some(b) = &self.badge {
            b.setFrame(ns_rect(f.badge));
        }
    }
}

impl Built {
    /// Measure (and draw a card at) `width`.
    fn shape(&mut self, width: f64) -> Shape {
        match &mut self.parts {
            Parts::Bubble(b) => Shape::Bubble {
                side: b.side,
                text: b.size(width),
                meta_w: b.meta_w,
                badge: b.badge.is_some(),
            },
            Parts::Card { card, .. } => {
                let w = full_width(width);
                if let Owned::Card { card: c, ui, .. } = &self.row
                    && card.as_ref().is_none_or(|e| (e.width() - w).abs() >= 0.5)
                {
                    redraw(&self.view, card, c, &ui.get(), w);
                }
                Shape::Full { h: card.as_ref().map_or(0.0, |e| e.height()) }
            }
            Parts::Divider(_) => Shape::Full { h: DIVIDER_H },
        }
    }

    fn place(&self, f: &Frames) {
        self.view.setFrame(ns_rect(f.row));
        let inside = Rect::new(0.0, 0.0, f.row.w, f.row.h);
        match &self.parts {
            Parts::Bubble(b) => b.place(f),
            Parts::Card { back, .. } => back.setFrame(ns_rect(inside)),
            Parts::Divider(l) => l.setFrame(ns_rect(Rect::new(0.0, 1.0, f.row.w, DIVIDER_H - 2.0))),
        }
    }
}

/// Draw `card` into `view` at `width`, carrying the inputs of the drawing in `slot`.
fn redraw(
    view: &NSView,
    slot: &mut Option<Box<Embedded>>,
    card: &crate::core::card::Card,
    ui: &crate::platform::hud::CardUi,
    width: f64,
) {
    let new = Embedded::draw(card, ui, width, slot.as_deref());
    if let Some(old) = slot.take() {
        old.view().removeFromSuperview();
    }
    view.addSubview(new.view());
    *slot = Some(Box::new(new));
}

/// A surface's rows area.
pub(super) struct Transcript {
    scroll: Retained<NSScrollView>,
    doc: Retained<Flipped>,
    rows: RefCell<Vec<Built>>,
    /// Rows from the last `set`, waiting for the coalesced `flush`.
    pending: RefCell<Option<Vec<Owned>>>,
    coalesce: Cell<Coalesce>,
}

impl Transcript {
    pub(super) fn new(mtm: MainThreadMarker) -> Transcript {
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::ZERO);
        scroll.setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        let doc = flipped(mtm);
        scroll.setDocumentView(Some(&doc));
        let none = RefCell::default();
        Transcript {
            scroll,
            doc,
            rows: none,
            pending: RefCell::default(),
            coalesce: Cell::default(),
        }
    }

    pub(super) fn view(&self) -> &NSView {
        &self.scroll
    }

    fn visible(&self) -> NSSize {
        self.scroll.contentSize()
    }

    fn at_bottom(&self) -> bool {
        let offset = self.scroll.contentView().bounds().origin.y;
        rows::at_bottom(offset, self.visible().height, self.doc.frame().size.height)
    }

    fn scroll_to_bottom(&self) {
        let clip = self.scroll.contentView();
        let y = rows::bottom(self.visible().height, self.doc.frame().size.height);
        clip.scrollToPoint(NSPoint::new(0.0, y));
        self.scroll.reflectScrolledClipView(&clip);
    }

    /// Lay every row out for the current size, keeping the bottom in view if it was.
    fn layout(&self, pinned: bool) {
        let size = self.visible();
        let mut rows = self.rows.borrow_mut();
        let shapes: Vec<Shape> = rows.iter_mut().map(|r| r.shape(size.width)).collect();
        let (frames, doc_h) = rows::stack(size.width, size.height, &shapes);
        self.doc.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(size.width, doc_h)));
        for (r, f) in rows.iter().zip(&frames) {
            r.place(f);
        }
        drop(rows);
        if pinned {
            self.scroll_to_bottom();
        }
    }

    /// Put the rows area at `r` (top-down in the content view).
    pub(super) fn set_frame(&self, r: Rect) {
        let pinned = self.at_bottom();
        self.scroll.setFrame(ns_rect(r));
        self.layout(pinned);
    }

    /// Queue `rows` for the next `flush`; true when the caller must schedule one.
    pub(super) fn set(&self, rows: Vec<Owned>) -> bool {
        self.pending.replace(Some(rows));
        let mut c = self.coalesce.get();
        let arm = c.request();
        self.coalesce.set(c);
        arm
    }

    /// Show the queued rows: unchanged rows keep their views, the rest are built anew.
    pub(super) fn flush(&self) {
        let mut c = self.coalesce.get();
        c.done();
        self.coalesce.set(c);
        let Some(new) = self.pending.take() else { return };
        let pinned = self.at_bottom();
        let mtm = MainThreadMarker::from(&*self.doc);
        let mut old: Vec<Option<Built>> = self.rows.take().into_iter().map(Some).collect();
        let idents: Vec<_> = old.iter().flatten().map(|b| b.row.ident()).collect();
        let fresh: Vec<_> = new.iter().map(Owned::ident).collect();
        let plans = rows::diff(&idents, &fresh);
        let built: Vec<Built> = new
            .into_iter()
            .zip(plans)
            .map(|(row, plan)| match plan {
                Plan::Keep(i) => old[i].take().unwrap_or_else(|| build(mtm, &self.doc, row)),
                Plan::Build => build(mtm, &self.doc, row),
            })
            .collect();
        for gone in old.into_iter().flatten() {
            gone.view.removeFromSuperview();
        }
        self.rows.replace(built);
        self.layout(pinned);
    }

    /// Whether a card row was drawn into the view at `host`.
    pub(super) fn has_card(&self, host: usize) -> bool {
        self.rows.borrow().iter().any(|r| card_at(r, host).is_some())
    }

    /// The card id, action and values of a press of button `tag` on the card at `host`.
    pub(super) fn press(
        &self,
        host: usize,
        tag: isize,
    ) -> Option<(String, String, Result<String, String>)> {
        let rows = self.rows.borrow();
        let (row, embedded) = rows.iter().find_map(|r| card_at(r, host).map(|e| (r, e)))?;
        let Owned::Card { card, .. } = &row.row else { return None };
        let (action, values) = embedded.press(tag)?;
        Some((card.id.clone(), action, values))
    }

    /// Redraw the card at `host` with `error` as its error line (the press did not go out).
    pub(super) fn show_error(&self, host: usize, error: &str) {
        let mut rows = self.rows.borrow_mut();
        let Some(r) = rows.iter_mut().find(|r| card_at(r, host).is_some()) else { return };
        if let (Owned::Card { card, ui, .. }, Parts::Card { card: slot, .. }) =
            (&mut r.row, &mut r.parts)
        {
            *ui = ui.with_error(error);
            let w = slot.as_ref().map_or(0.0, |e| e.width());
            redraw(&r.view, slot, card, &ui.get(), w);
        }
        drop(rows);
        self.layout(self.at_bottom());
    }
}

fn card_at(r: &Built, host: usize) -> Option<&Embedded> {
    match &r.parts {
        Parts::Card { card: Some(e), .. } if e.key() == host => Some(e),
        _ => None,
    }
}
