//! The plain text card (the `message` module's): an icon, a bold header, the time, an
//! optional context line, a wrapping body and a hint line.

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSColor, NSFont, NSFontWeightSemibold, NSImage, NSImageScaling, NSImageView, NSTextAlignment,
    NSTextField, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::view::{CLOSE_ROOM, label, ns, rect};

const PAD: f64 = 14.0;
const HEADER_H: f64 = 18.0;
const LINE_H: f64 = 15.0;
const MAX_BODY_H: f64 = 360.0;
const TIME_W: f64 = 66.0;

/// What a text card shows.
#[derive(Clone, Copy, Debug)]
pub struct TextCard<'a> {
    /// Bold, top left: the sender or the message title.
    pub header: &'a str,
    /// Top right, e.g. "14:03".
    pub time: &'a str,
    /// One secondary line under the header (e.g. the question this replies to); empty: none.
    pub context: &'a str,
    pub body: &'a str,
    /// Opened by a click.
    pub link: Option<&'a str>,
    /// A placeholder waiting for its reply: dimmed body and an hourglass.
    pub pending: bool,
}

/// The hint line under the body.
pub fn hint(card: &TextCard) -> String {
    match (card.pending, card.link) {
        (true, _) => "Waiting for a reply…  ·  esc to dismiss".into(),
        (false, Some(link)) => format!("Click to open {}  ·  esc to dismiss", host(link)),
        (false, None) => "Click or esc to dismiss".into(),
    }
}

/// `https://example.com/a/b` → `example.com`; anything else as is.
fn host(link: &str) -> &str {
    let rest = link.split_once("://").map_or(link, |(_, r)| r);
    rest.split('/').next().filter(|h| !h.is_empty()).unwrap_or(link)
}

/// Fill `into` with `card` at `width`; returns the card's height.
pub fn render(mtm: MainThreadMarker, into: &NSView, card: &TextCard, width: f64) -> f64 {
    let inner = width - 2.0 * PAD;
    let symbol = if card.pending { "hourglass" } else { "bubble.left.fill" };
    let icon =
        NSImageView::initWithFrame(NSImageView::alloc(mtm), rect(PAD, PAD + 1.0, 16.0, 16.0));
    icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
    icon.setImage(
        NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(symbol), None).as_deref(),
    );
    into.addSubview(&icon);

    // SAFETY: NSFontWeightSemibold is an immutable framework constant, set at load time.
    let semibold = unsafe { NSFontWeightSemibold };
    let header =
        label(mtm, &NSFont::systemFontOfSize_weight(13.0, semibold), &NSColor::labelColor());
    header.setStringValue(&ns(card.header));
    let time_x = width - CLOSE_ROOM - TIME_W;
    header.setFrame(rect(PAD + 22.0, PAD, time_x - PAD - 26.0, HEADER_H));
    into.addSubview(&header);
    let time = label(mtm, &NSFont::systemFontOfSize(11.0), &NSColor::tertiaryLabelColor());
    time.setAlignment(NSTextAlignment::Right);
    time.setStringValue(&ns(card.time));
    time.setFrame(rect(time_x, PAD + 2.0, TIME_W, LINE_H));
    into.addSubview(&time);

    let mut y = PAD + HEADER_H + 4.0;
    if !card.context.is_empty() {
        let context = label(mtm, &NSFont::systemFontOfSize(11.0), &NSColor::secondaryLabelColor());
        context.setStringValue(&ns(card.context));
        context.setFrame(rect(PAD, y, inner, LINE_H));
        into.addSubview(&context);
        y += LINE_H + 4.0;
    }

    let body = NSTextField::wrappingLabelWithString(&ns(card.body), mtm);
    body.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    body.setSelectable(false);
    if let Some(cell) = body.cell() {
        cell.setTruncatesLastVisibleLine(true);
    }
    let color = if card.pending { NSColor::secondaryLabelColor() } else { NSColor::labelColor() };
    body.setTextColor(Some(&color));
    body.setPreferredMaxLayoutWidth(inner);
    let big = NSRect::new(NSPoint::ZERO, NSSize::new(inner, 10_000.0));
    let measured = body.cell().map_or(LINE_H, |c| c.cellSizeForBounds(big).height);
    let body_h = measured.ceil().min(MAX_BODY_H);
    body.setFrame(rect(PAD, y + 2.0, inner, body_h));
    into.addSubview(&body);
    y += 2.0 + body_h + 8.0;

    let hint_label = label(mtm, &NSFont::systemFontOfSize(11.0), &NSColor::tertiaryLabelColor());
    hint_label.setStringValue(&ns(&hint(card)));
    hint_label.setFrame(rect(PAD, y, inner, LINE_H));
    into.addSubview(&hint_label);
    y + LINE_H + PAD - 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hint_names_the_link_host() {
        let card = |pending, link| TextCard {
            header: "KOTA",
            time: "",
            context: "",
            body: "",
            link,
            pending,
        };
        assert_eq!(hint(&card(false, None)), "Click or esc to dismiss");
        assert_eq!(
            hint(&card(false, Some("https://github.com/jayminwest/flick/pull/2"))),
            "Click to open github.com  ·  esc to dismiss"
        );
        assert_eq!(
            hint(&card(true, Some("https://x.y"))),
            "Waiting for a reply…  ·  esc to dismiss"
        );
        assert_eq!(host("mailto:a@b.c"), "mailto:a@b.c");
        assert_eq!(host("https://"), "https://");
    }
}
