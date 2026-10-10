//! The panel's result rows, empty-state text and footer.

use std::path::Path;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBorderType, NSBox, NSBoxType, NSColor, NSFont, NSImage, NSImageScaling, NSImageView,
    NSLineBreakMode, NSScrollView, NSTextAlignment, NSTextField, NSTextView, NSView, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::scroll::{self, Before, Extent, TextScroll};
use super::{FOOTER_H, H, LIST_PAD, ROW_H, SEARCH_H, Ui, VISIBLE_ROWS, W, with_ui};

/// A row's icon: a file's Finder icon, or an SF Symbol name.
pub enum Icon<'a> {
    File(&'a Path),
    Symbol(&'a str),
}

/// A row's status tint for a symbol icon (`core::Tone`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Neutral,
    Ok,
    Warn,
    Error,
}

pub struct Row<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub accessory: &'a str,
    pub icon: Icon<'a>,
    pub tone: Tone,
}

/// The tint of a symbol icon: the system status colors, which follow light and dark mode.
fn tint(tone: Tone) -> Retained<NSColor> {
    match tone {
        Tone::Neutral => NSColor::secondaryLabelColor(),
        Tone::Ok => NSColor::systemGreenColor(),
        Tone::Warn => NSColor::systemOrangeColor(),
        Tone::Error => NSColor::systemRedColor(),
    }
}

/// Everything the panel shows for a list.
pub struct Frame<'a> {
    /// A read-only title in place of the search field, which then takes no typing.
    pub title: Option<&'a str>,
    /// At most `VISIBLE_ROWS` rows, top to bottom.
    pub rows: &'a [Row<'a>],
    /// Index into `rows` of the highlighted row.
    pub selected: Option<usize>,
    /// Centered text, for when there are no rows.
    pub empty: &'a str,
    pub footer: &'a str,
    /// Right-aligned footer text: what Return does.
    pub action: &'a str,
    /// Read-only text under the rows, word-wrapped in a fixed-width font, scrollable (mouse,
    /// `scroll_text`). Empty: none.
    pub text: &'a str,
    /// New `text` starts at its end and follows it while the user is at the end.
    pub text_tail: bool,
}

pub(super) struct RowViews {
    pub(super) view: Retained<NSView>,
    bg: Retained<NSBox>,
    icon: Retained<NSImageView>,
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
    accessory: Retained<NSTextField>,
}

pub(super) fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// A frame measured from the top-left of a parent of height `parent_h`.
pub(super) fn top_rect(parent_h: f64, x: f64, top: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, parent_h - top - h), NSSize::new(w, h))
}

pub(super) fn label(mtm: MainThreadMarker, size: f64, color: &NSColor) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(""), mtm);
    l.setFont(Some(&NSFont::systemFontOfSize(size)));
    l.setTextColor(Some(color));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

pub(super) fn separator(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSBox> {
    let b = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    b.setBoxType(NSBoxType::Separator);
    b
}

/// The read-only text block under the rows (`Frame::text`): a non-editable, non-selectable
/// text view in a scroll view, hidden until a frame has text.
pub(super) struct TextViews {
    pub(super) scroll: Retained<NSScrollView>,
    view: Retained<NSTextView>,
    /// One line's height, in points.
    line: f64,
}

pub(super) fn make_text(mtm: MainThreadMarker) -> TextViews {
    let font = NSFont::userFixedPitchFontOfSize(12.0)
        .unwrap_or_else(|| NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0));
    let scroll = NSTextView::scrollableTextView(mtm);
    scroll.setDrawsBackground(false);
    scroll.setBorderType(NSBorderType::NoBorder);
    scroll.setHasVerticalScroller(true);
    scroll.setAutohidesScrollers(true);
    // `scrollableTextView` makes an NSTextView its document view; the fallback never runs.
    let view = scroll.documentView().and_then(|v| v.downcast::<NSTextView>().ok());
    let view = view.unwrap_or_else(|| {
        let t = NSTextView::initWithFrame(NSTextView::alloc(mtm), NSRect::ZERO);
        scroll.setDocumentView(Some(&t));
        t
    });
    view.setEditable(false);
    // Not selectable: a click would take the keyboard from the panel and its keys.
    view.setSelectable(false);
    view.setRichText(false);
    view.setDrawsBackground(false);
    view.setFont(Some(&font));
    view.setTextColor(Some(&NSColor::labelColor()));
    view.setTextContainerInset(NSSize::ZERO);
    // SAFETY: the text view's own container, used on the main thread.
    if let Some(c) = unsafe { view.textContainer() } {
        c.setLineFragmentPadding(0.0);
    }
    // SAFETY: the text view's own layout manager, used on the main thread.
    let line =
        unsafe { view.layoutManager() }.map_or(15.0, |lm| lm.defaultLineHeightForFont(&font));
    scroll.setHidden(true);
    TextViews { scroll, view, line }
}

impl TextViews {
    fn offset(&self) -> f64 {
        self.scroll.contentView().bounds().origin.y
    }

    /// Lay the text out and measure it.
    fn extent(&self) -> Extent {
        self.view.sizeToFit();
        Extent {
            visible: self.scroll.contentSize().height,
            content: self.view.frame().size.height,
            line: self.line,
        }
    }

    fn scroll_to(&self, offset: f64) {
        let clip = self.scroll.contentView();
        clip.scrollToPoint(NSPoint::new(0.0, offset));
        self.scroll.reflectScrolledClipView(&clip);
    }

    /// Show `text` at `frame`, scrolled as `scroll::after_change` says.
    fn show(&self, frame: NSRect, text: &str, tail: bool) {
        let old = self.view.string();
        let before = Before {
            empty: old.length() == 0,
            at_bottom: self.extent().at_bottom(self.offset()),
            offset: self.offset(),
        };
        self.scroll.setFrame(frame);
        if old.to_string() != text {
            self.view.setString(&ns(text));
        }
        self.scroll.setHidden(text.is_empty());
        self.scroll_to(scroll::after_change(before, tail, self.extent()));
    }
}

/// Scroll the read-only text block `by`. False when no text shows.
pub fn scroll_text(by: TextScroll) -> bool {
    with_ui(|ui| {
        let t = &ui.text;
        if t.scroll.isHidden() {
            return false;
        }
        t.scroll_to(scroll::scrolled(by, t.offset(), t.extent()));
        true
    })
    .unwrap_or(false)
}

pub(super) fn make_row(mtm: MainThreadMarker, index: usize) -> RowViews {
    let top = SEARCH_H + LIST_PAD + index as f64 * ROW_H;
    let view = NSView::initWithFrame(NSView::alloc(mtm), top_rect(H, 8.0, top, W - 16.0, ROW_H));

    let bg = NSBox::initWithFrame(
        NSBox::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(W - 16.0, ROW_H)),
    );
    bg.setBoxType(NSBoxType::Custom);
    bg.setBorderWidth(0.0);
    bg.setCornerRadius(8.0);
    bg.setFillColor(&NSColor::labelColor().colorWithAlphaComponent(0.1));
    bg.setTransparent(true);

    let icon = NSImageView::initWithFrame(
        NSImageView::alloc(mtm),
        top_rect(ROW_H, 12.0, 11.0, 22.0, 22.0),
    );
    icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);

    let title = label(mtm, 14.0, &NSColor::labelColor());
    let subtitle = label(mtm, 13.0, &NSColor::secondaryLabelColor());
    let accessory = label(mtm, 13.0, &NSColor::tertiaryLabelColor());
    accessory.setAlignment(NSTextAlignment::Right);
    accessory.setFrame(top_rect(ROW_H, W - 16.0 - 12.0 - 160.0, 13.0, 160.0, 18.0));

    for v in [&*bg as &NSView, &icon, &title, &subtitle, &accessory] {
        view.addSubview(v);
    }
    view.setHidden(true);
    RowViews { view, bg, icon, title, subtitle, accessory }
}

fn icon_image(ui: &Ui, icon: &Icon) -> Option<Retained<NSImage>> {
    let key = match icon {
        Icon::File(p) => p.display().to_string(),
        Icon::Symbol(s) => format!("sf:{s}"),
    };
    if let Some(img) = ui.icons.borrow().get(&key) {
        return Some(img.clone());
    }
    let img = match icon {
        Icon::File(p) => NSWorkspace::sharedWorkspace().iconForFile(&ns(&p.display().to_string())),
        Icon::Symbol(s) => {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(s), None).or_else(
                || NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns("app"), None),
            )?
        }
    };
    ui.icons.borrow_mut().insert(key, img.clone());
    Some(img)
}

pub fn render(frame: &Frame) {
    with_ui(|ui| {
        super::form::list_mode(ui, frame.title);
        let visible = frame.rows.iter().enumerate().take(VISIBLE_ROWS);
        let mut shown = 0;
        for (row, (index, item)) in ui.rows.iter().zip(visible) {
            shown += 1;
            row.view.setHidden(false);
            row.bg.setTransparent(Some(index) != frame.selected);

            row.icon.setImage(icon_image(ui, &item.icon).as_deref());
            let tint = matches!(item.icon, Icon::Symbol(_)).then(|| tint(item.tone));
            row.icon.setContentTintColor(tint.as_deref());

            let title_x = 46.0;
            let max_text = W - 16.0 - title_x - 12.0 - 170.0;
            row.title.setStringValue(&ns(item.title));
            let title_w = row.title.cell().map_or(0.0, |c| c.cellSize().width).min(max_text).ceil();
            row.title.setFrame(top_rect(ROW_H, title_x, 13.0, title_w, 18.0));

            row.subtitle.setStringValue(&ns(item.subtitle));
            let sub_w = (max_text - title_w - 8.0).max(0.0);
            row.subtitle.setFrame(top_rect(ROW_H, title_x + title_w + 8.0, 13.5, sub_w, 17.0));

            row.accessory.setStringValue(&ns(item.accessory));
        }
        for row in &ui.rows[shown..] {
            row.view.setHidden(true);
        }
        let top = SEARCH_H + LIST_PAD + shown as f64 * ROW_H + 6.0;
        let h = scroll::whole_lines(H - FOOTER_H - top - 8.0, ui.text.line);
        let area = top_rect(H, 20.0, top, W - 40.0, h);
        ui.text.show(area, frame.text, frame.text_tail);

        ui.empty.setStringValue(&ns(frame.empty));
        ui.footer_left.setStringValue(&ns(frame.footer));
        ui.footer_action.setStringValue(&ns(frame.action));
    });
}
