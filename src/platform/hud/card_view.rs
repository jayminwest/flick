//! The card renderer: draws a `core::card::Card` into a card's content view. A title with a
//! state symbol, the blocks top to bottom (text, kv, list, progress, choice, field), a status
//! line (pending spinner, error, done), and the action buttons, right-aligned; or, while a
//! shell action waits for confirmation, the exact command with Cancel and Run. Sizes, labels
//! and rules come from the pure `card_layout`; this file only places `AppKit` views.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{
    NSButtonType, NSColor, NSControlSize, NSControlStateValueOff, NSControlStateValueOn, NSFont,
    NSFontAttributeName, NSFontWeightSemibold, NSForegroundColorAttributeName, NSImage,
    NSImageScaling, NSImageView, NSMenuItem, NSProgressIndicator, NSProgressIndicatorStyle,
    NSTextField, NSTextFieldBezelStyle, NSView,
};
use objc2_foundation::{NSAttributedString, NSDictionary, NSPoint, NSRect, NSSize};

use super::card_layout::{
    self as lay, BAR_H, BUTTON_GAP, BUTTON_H, CHOICE_ROW_H, CardUi, ChoiceStyle, FIELD_H, GAP,
    MAX_TEXT_H, MAX_VALUE_H, MULTI_H, NO_CHOICE, PAD, POPUP_H, SMALL_H, SPINNER, Tone,
};
use super::controls::{
    Button, CANCEL_TAG, Controls, Ctl, Field, MULTILINE, PopUp, RUN_TAG, Target, target,
};
use super::view::{CLOSE_ROOM, ContentView, label, ns, rect};
use crate::core::card::{Action, Block, Card, Input, Opt, Style};
use crate::core::markup;

/// What drawing one card needs besides the card.
struct Pen<'a> {
    mtm: MainThreadMarker,
    into: &'a NSView,
    target: Retained<Target>,
    /// Inner width: the card's width less the padding.
    inner: f64,
    enabled: bool,
}

/// Fill `into` with `card` in state `ui` at `width`, its inputs showing `values` (by input
/// id; missing ids show the card's own). Returns the card's height and its live controls.
pub fn render(
    mtm: MainThreadMarker,
    into: &NSView,
    card: &Card,
    ui: &CardUi,
    values: &[(String, Input)],
    width: f64,
) -> (f64, Controls) {
    let status = lay::status(card.state, ui);
    let confirm = lay::confirming(card, ui);
    let pen =
        Pen { mtm, into, target: target(mtm), inner: width - 2.0 * PAD, enabled: status.enabled };
    let mut y = header(&pen, card, status.symbol, width);
    let mut inputs = Vec::new();
    for b in &card.blocks {
        y = block(&pen, b, y + GAP, values, &mut inputs);
    }
    if confirm.is_none()
        && let Some((tone, line)) = &status.line
    {
        y = status_line(&pen, *tone, line, status.spinner, y + GAP);
    }
    let actions = if let Some((_, cmd)) = confirm {
        y = confirm_step(&pen, cmd, y + GAP);
        vec![]
    } else {
        y = buttons(&pen, &card.actions, y + GAP);
        card.actions.iter().map(|a| a.id.clone()).collect()
    };
    let controls = Controls {
        inputs,
        initial: card.inputs(),
        actions,
        confirm: confirm.map(|(a, _)| a.id.clone()),
        card: card.clone(),
        pending: ui.pending,
    };
    (y + PAD, controls)
}

/// The state symbol and the bold title; returns the y under them.
fn header(pen: &Pen, card: &Card, symbol: &str, width: f64) -> f64 {
    let icon =
        NSImageView::initWithFrame(NSImageView::alloc(pen.mtm), rect(PAD, PAD + 1.0, 16.0, 16.0));
    icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
    icon.setImage(
        NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(symbol), None).as_deref(),
    );
    pen.into.addSubview(&icon);
    // SAFETY: NSFontWeightSemibold is an immutable framework constant, set at load time.
    let semibold = unsafe { NSFontWeightSemibold };
    let title =
        label(pen.mtm, &NSFont::systemFontOfSize_weight(13.0, semibold), &NSColor::labelColor());
    title.setStringValue(&ns(&card.title));
    title.setFrame(rect(PAD + 22.0, PAD, width - PAD - 22.0 - CLOSE_ROOM, lay::TITLE_H));
    pen.into.addSubview(&title);
    PAD + lay::TITLE_H
}

/// A wrapping label at `at` (left, top, width) and at most `max_h` tall; returns its height.
fn wrapped(
    pen: &Pen,
    text: &str,
    font: &NSFont,
    color: &NSColor,
    at: (f64, f64, f64),
    max_h: f64,
) -> f64 {
    let (left, top, w) = at;
    let view = NSTextField::wrappingLabelWithString(&ns(text), pen.mtm);
    view.setFont(Some(font));
    view.setSelectable(false);
    view.setTextColor(Some(color));
    if let Some(cell) = view.cell() {
        cell.setTruncatesLastVisibleLine(true);
    }
    view.setPreferredMaxLayoutWidth(w);
    let big = NSRect::new(NSPoint::ZERO, NSSize::new(w, 100_000.0));
    let h = view.cell().map_or(SMALL_H, |c| c.cellSizeForBounds(big).height).ceil().min(max_h);
    view.setFrame(rect(left, top, w, h));
    pen.into.addSubview(&view);
    h
}

/// A one-line secondary label (a block's label); returns the y under it.
fn caption(pen: &Pen, text: &str, y: f64) -> f64 {
    let l = label(pen.mtm, &NSFont::systemFontOfSize(11.0), &NSColor::secondaryLabelColor());
    l.setStringValue(&ns(text));
    l.setFrame(rect(PAD, y, pen.inner, SMALL_H));
    pen.into.addSubview(&l);
    y + SMALL_H + 3.0
}

/// Draw block `b` at `y`; returns the y under it.
fn block(
    pen: &Pen,
    b: &Block,
    y: f64,
    values: &[(String, Input)],
    inputs: &mut Vec<(String, Ctl)>,
) -> f64 {
    let body = NSFont::systemFontOfSize(13.0);
    let ink = NSColor::labelColor();
    match b {
        Block::Text { md } => {
            y + wrapped(pen, &markup::plain(md), &body, &ink, (PAD, y, pen.inner), MAX_TEXT_H)
        }
        Block::Kv { items } => {
            let small = NSFont::systemFontOfSize(12.0);
            let keys: Vec<Retained<NSTextField>> = items
                .iter()
                .map(|p| {
                    let key = label(pen.mtm, &small, &NSColor::secondaryLabelColor());
                    key.setStringValue(&ns(&p.key));
                    key
                })
                .collect();
            let fitted: Vec<f64> = keys.iter().map(|k| k.fittingSize().width).collect();
            let kw = lay::key_width(&fitted, pen.inner);
            let mut y = y;
            for (p, key) in items.iter().zip(&keys) {
                key.setFrame(rect(PAD, y, kw, SMALL_H));
                pen.into.addSubview(key);
                let at = (PAD + kw + 8.0, y, pen.inner - kw - 8.0);
                y += wrapped(pen, &p.value, &small, &ink, at, MAX_VALUE_H).max(SMALL_H) + 3.0;
            }
            y - 3.0
        }
        Block::List { items, ordered } => {
            let mut y = y;
            for (i, item) in items.iter().enumerate() {
                let row = lay::list_row(i, item, *ordered);
                y += wrapped(pen, &row, &body, &ink, (PAD, y, pen.inner), MAX_TEXT_H) + 2.0;
            }
            y - 2.0
        }
        Block::Progress { value, label } => progress(pen, *value, label.as_deref(), y),
        Block::Choice { id, label, options, multi, .. } => {
            let y = label.as_deref().map_or(y, |l| caption(pen, l, y));
            let value = values.iter().find(|(i, _)| i == id).map(|(_, v)| v);
            let (ctl, h) = choice(pen, options, *multi, value, y);
            inputs.push((id.clone(), ctl));
            y + h
        }
        Block::Field { id, label, placeholder, multiline, .. } => {
            let y = label.as_deref().map_or(y, |l| caption(pen, l, y));
            let text = match values.iter().find(|(i, _)| i == id) {
                Some((_, Input::Text(t))) => t.as_str(),
                _ => "",
            };
            let (ctl, h) = field(pen, placeholder.as_deref(), text, *multiline, y);
            inputs.push((id.clone(), ctl));
            y + h
        }
    }
}

fn progress(pen: &Pen, value: Option<f64>, label: Option<&str>, y: f64) -> f64 {
    let y = caption(pen, &lay::progress_text(value, label), y);
    let bar = NSProgressIndicator::initWithFrame(
        NSProgressIndicator::alloc(pen.mtm),
        rect(PAD, y, pen.inner, BAR_H),
    );
    bar.setStyle(NSProgressIndicatorStyle::Bar);
    bar.setControlSize(NSControlSize::Small);
    if let Some(v) = value {
        bar.setIndeterminate(false);
        bar.setMinValue(0.0);
        bar.setMaxValue(1.0);
        bar.setDoubleValue(v);
    } else {
        bar.setIndeterminate(true);
        // SAFETY: nil is a valid sender.
        unsafe { bar.startAnimation(None) };
    }
    pen.into.addSubview(&bar);
    y + BAR_H
}

/// A choice's controls at `y`, showing `value`; returns them and their height.
fn choice(pen: &Pen, options: &[Opt], multi: bool, value: Option<&Input>, y: f64) -> (Ctl, f64) {
    let style = lay::choice_style(multi, options.len());
    let h = lay::choice_height(style, options.len());
    let picked = |id: &str| match value {
        Some(Input::One(Some(s))) => s == id,
        Some(Input::Many(ids)) => ids.iter().any(|s| s == id),
        _ => false,
    };
    if style == ChoiceStyle::Popup {
        // SAFETY: NSPopUpButton's designated initializer, with its argument types.
        let view: Retained<PopUp> = unsafe {
            msg_send![PopUp::alloc(pen.mtm), initWithFrame: rect(PAD, y, pen.inner, POPUP_H), pullsDown: false]
        };
        let blank = !options.iter().any(|o| picked(&o.id));
        let titles =
            blank.then_some(NO_CHOICE).into_iter().chain(options.iter().map(|o| o.label.as_str()));
        if let Some(menu) = view.menu() {
            for title in titles {
                // SAFETY: no action selector; an empty key equivalent.
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(pen.mtm),
                        &ns(title),
                        None,
                        &ns(""),
                    )
                };
                menu.addItem(&item);
            }
        }
        let at = if blank { 0 } else { options.iter().position(|o| picked(&o.id)).unwrap_or(0) };
        view.selectItemAtIndex(at as isize);
        view.setEnabled(pen.enabled);
        pen.into.addSubview(&view);
        let ids = options.iter().map(|o| o.id.clone()).collect();
        return (Ctl::Popup { view, ids, blank }, h);
    }
    // Radio buttons group by superview: each choice gets its own.
    // SAFETY: `initWithFrame:` is NSView's designated initializer; it returns a +1 object.
    let group: Retained<ContentView> = unsafe {
        msg_send![ContentView::alloc(pen.mtm), initWithFrame: rect(PAD, y, pen.inner, h)]
    };
    let kind =
        if style == ChoiceStyle::Radios { NSButtonType::Radio } else { NSButtonType::Switch };
    let mut buttons = Vec::new();
    for (i, o) in options.iter().enumerate() {
        let b = button(pen, &o.label, rect(0.0, i as f64 * CHOICE_ROW_H, pen.inner, CHOICE_ROW_H));
        b.setButtonType(kind);
        // SAFETY: `choose:` is a method of `Target`, which `target` keeps alive.
        unsafe { b.setAction(Some(sel!(choose:))) };
        b.setState(if picked(&o.id) { NSControlStateValueOn } else { NSControlStateValueOff });
        b.setEnabled(pen.enabled);
        group.addSubview(&b);
        buttons.push((o.id.clone(), b));
    }
    pen.into.addSubview(&group);
    let ctl =
        if style == ChoiceStyle::Radios { Ctl::Radios(buttons) } else { Ctl::Checks(buttons) };
    (ctl, h)
}

/// A text field at `y` with `text`; returns it and its height.
fn field(pen: &Pen, placeholder: Option<&str>, text: &str, multiline: bool, y: f64) -> (Ctl, f64) {
    let h = if multiline { MULTI_H } else { FIELD_H };
    // SAFETY: `initWithFrame:` is NSTextField's designated initializer; it returns a +1 object.
    let f: Retained<Field> =
        unsafe { msg_send![Field::alloc(pen.mtm), initWithFrame: rect(PAD, y, pen.inner, h)] };
    f.setBezeled(true);
    f.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
    f.setEditable(true);
    f.setSelectable(true);
    f.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    f.setStringValue(&ns(text));
    if let Some(p) = placeholder {
        f.setPlaceholderString(Some(&ns(p)));
    }
    f.setUsesSingleLineMode(!multiline);
    if let Some(cell) = f.cell() {
        cell.setWraps(multiline);
        cell.setScrollable(!multiline);
    }
    if multiline {
        f.setTag(MULTILINE);
    }
    // SAFETY: `target` keeps the delegate alive as long as the process (the field holds it
    // weakly).
    unsafe { f.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*pen.target))) };
    f.setEnabled(pen.enabled);
    pen.into.addSubview(&f);
    (Ctl::Text(f), h)
}

/// The pending, error or done line at `y`; returns the y under it.
fn status_line(pen: &Pen, tone: Tone, line: &str, spinner: bool, y: f64) -> f64 {
    let mut x = PAD;
    if spinner {
        let s = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(pen.mtm),
            rect(PAD, y + 0.5, SPINNER, SPINNER),
        );
        s.setStyle(NSProgressIndicatorStyle::Spinning);
        s.setControlSize(NSControlSize::Small);
        // SAFETY: nil is a valid sender.
        unsafe { s.startAnimation(None) };
        pen.into.addSubview(&s);
        x += SPINNER + 6.0;
    }
    let color = match tone {
        Tone::Muted => NSColor::secondaryLabelColor(),
        Tone::Error => NSColor::systemRedColor(),
    };
    let font = NSFont::systemFontOfSize(12.0);
    y + wrapped(pen, line, &font, &color, (x, y, pen.inner - (x - PAD)), MAX_VALUE_H).max(SMALL_H)
}

/// The confirm step: what will run, the exact command (never cut), Cancel and Run.
fn confirm_step(pen: &Pen, cmd: &str, y: f64) -> f64 {
    // SAFETY: NSFontWeightSemibold is an immutable framework constant, set at load time.
    let semibold = unsafe { NSFontWeightSemibold };
    let bold = NSFont::systemFontOfSize_weight(12.0, semibold);
    let ink = NSColor::labelColor();
    let y = y + wrapped(
        pen,
        "Run this command on this Mac?",
        &bold,
        &ink,
        (PAD, y, pen.inner),
        SMALL_H,
    );
    let mono = NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0);
    let y = y + 4.0 + wrapped(pen, cmd, &mono, &ink, (PAD, y + 4.0, pen.inner), f64::INFINITY);
    let cancel = push(pen, "Cancel", CANCEL_TAG, true);
    let run = push(pen, "Run", RUN_TAG, true);
    style(&run, "Run", Style::Destructive);
    place_buttons(pen, &[cancel, run], y + GAP)
}

/// The action buttons at `y`, right-aligned and wrapping; returns the y under them.
fn buttons(pen: &Pen, actions: &[Action], y: f64) -> f64 {
    let views: Vec<Retained<Button>> = actions
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let b = push(pen, &a.label, i as isize, pen.enabled && a.enabled());
            style(&b, &a.label, a.style);
            b.setToolTip(lay::tooltip(a).map(|t| ns(&t)).as_deref());
            b
        })
        .collect();
    if views.is_empty() { y - GAP } else { place_buttons(pen, &views, y) }
}

/// Mark an enabled primary button (semibold accent title) or destructive one (red title).
/// Bezel colors do not show in a panel that is not key, so the title carries the style.
fn style(b: &Button, title: &str, style: Style) {
    let (color, weight) = match style {
        Style::Default => return,
        // SAFETY: NSFontWeightSemibold is an immutable framework constant, set at load time.
        Style::Primary => (NSColor::controlAccentColor(), unsafe { NSFontWeightSemibold }),
        Style::Destructive => (NSColor::systemRedColor(), 0.0),
    };
    if !b.isEnabled() {
        return;
    }
    let font = NSFont::systemFontOfSize_weight(13.0, weight);
    let (font, color): (&AnyObject, &AnyObject) = (font.as_ref(), color.as_ref());
    // SAFETY: the attribute keys are immutable framework constants, set at load time.
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let attrs = NSDictionary::from_slices(&keys, &[font, color]);
    // SAFETY: the font attribute is an NSFont and the color attribute an NSColor.
    let text = unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &ns(title),
            Some(&attrs),
        )
    };
    b.setAttributedTitle(&text);
    b.setHasDestructiveAction(style == Style::Destructive);
}

/// A push button that presses `tag`.
fn push(pen: &Pen, title: &str, tag: isize, enabled: bool) -> Retained<Button> {
    let b = button(pen, title, rect(0.0, 0.0, 0.0, BUTTON_H));
    b.setButtonType(NSButtonType::MomentaryPushIn);
    b.setBezelStyle(objc2_app_kit::NSBezelStyle::Push);
    // SAFETY: `press:` is a method of `Target`, which `target` keeps alive.
    unsafe { b.setAction(Some(sel!(press:))) };
    b.setTag(tag);
    b.setEnabled(enabled);
    b
}

/// A `Button` titled `title` at `frame`, targeting the card target.
fn button(pen: &Pen, title: &str, frame: NSRect) -> Retained<Button> {
    // SAFETY: `initWithFrame:` is NSButton's designated initializer; it returns a +1 object.
    let b: Retained<Button> = unsafe { msg_send![Button::alloc(pen.mtm), initWithFrame: frame] };
    b.setTitle(&ns(title));
    b.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    // SAFETY: `target` keeps the target alive for the life of the process.
    unsafe { b.setTarget(Some(&pen.target)) };
    b
}

/// Size `views` to fit and flow them right-aligned from `y`; returns the y under them.
fn place_buttons(pen: &Pen, views: &[Retained<Button>], y: f64) -> f64 {
    let widths: Vec<f64> = views
        .iter()
        .map(|b| {
            b.sizeToFit();
            lay::button_width(b.frame().size.width, pen.inner)
        })
        .collect();
    let (at, rows) = lay::flow(&widths, pen.inner);
    for ((b, w), (x, row)) in views.iter().zip(&widths).zip(at) {
        b.setFrame(rect(PAD + x, y + row as f64 * (BUTTON_H + BUTTON_GAP), *w, BUTTON_H));
        pen.into.addSubview(b);
    }
    y + lay::buttons_height(rows)
}
