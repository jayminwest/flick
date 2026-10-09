//! The panel's search-area modes and its form layout: a title in place of the search field,
//! labelled text fields (a multiline one is taller and wraps), and an error line. Which view
//! has keyboard focus follows the mode.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSColor, NSFocusRingType, NSFont, NSLineBreakMode, NSResponder, NSStandardKeyBindingResponding,
    NSTextAlignment, NSTextField, NSTextFieldBezelStyle, NSTextView, NSView,
};

use super::rows::{label, ns, top_rect};
use super::{Delegate, FOOTER_H, H, SEARCH_H, W, with_ui};

/// How many fields a form can show.
pub const FORM_FIELDS: usize = 6;
const FIELDS_TOP: f64 = SEARCH_H + 24.0;
const FIELD_ROW_H: f64 = 48.0;
const LABEL_W: f64 = 140.0;
const INPUT_X: f64 = 20.0 + LABEL_W + 16.0;
const INPUT_W: f64 = W - INPUT_X - 40.0;
const INPUT_H: f64 = 24.0;
/// How many field rows a multiline field takes.
const MULTILINE_ROWS: usize = 3;
const ERROR_H: f64 = 16.0;
// A full form's fields and error line fit above the footer.
const _: () = assert!(FIELDS_TOP + FORM_FIELDS as f64 * FIELD_ROW_H + ERROR_H <= H - FOOTER_H);

pub struct FormField<'a> {
    pub label: &'a str,
    pub value: &'a str,
    pub placeholder: &'a str,
    /// Taller and wrapping; Return inserts a newline.
    pub multiline: bool,
}

/// A form: a title in the search area, labelled text fields below it, an error line, a footer.
pub struct FormFrame<'a> {
    pub title: &'a str,
    /// Top to bottom, as many as fit in `FORM_FIELDS` rows (a multiline field takes
    /// `MULTILINE_ROWS`); the rest are not shown.
    pub fields: &'a [FormField<'a>],
    /// Index into `fields` of the field with keyboard focus.
    pub focused: usize,
    /// Shown in red under the fields; empty for none.
    pub error: &'a str,
    pub footer: &'a str,
    /// Right-aligned footer text: what Return does.
    pub action: &'a str,
}

/// What fills the search area, and so which view gets the keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    /// The editable search field.
    Search,
    /// A read-only title; the panel itself takes the keys.
    Title,
    /// A form; the focused field takes the keys.
    Form(usize),
}

pub(super) struct FormViews {
    title: Retained<NSTextField>,
    labels: Vec<Retained<NSTextField>>,
    inputs: Vec<Retained<NSTextField>>,
    /// Which inputs are multiline, as last drawn.
    multiline: Vec<Cell<bool>>,
    error: Retained<NSTextField>,
    pub(super) mode: Cell<Mode>,
}

pub(super) fn make_form(mtm: MainThreadMarker, root: &NSView, delegate: &Delegate) -> FormViews {
    let title = label(mtm, 20.0, &NSColor::labelColor());
    title.setFrame(top_rect(H, 20.0, 14.0, W - 40.0, 26.0));
    title.setHidden(true);
    root.addSubview(&title);

    let mut labels = Vec::with_capacity(FORM_FIELDS);
    let mut inputs = Vec::with_capacity(FORM_FIELDS);
    for i in 0..FORM_FIELDS {
        let top = FIELDS_TOP + i as f64 * FIELD_ROW_H;
        let l = label(mtm, 13.0, &NSColor::secondaryLabelColor());
        l.setAlignment(NSTextAlignment::Right);
        l.setFrame(top_rect(H, 20.0, top + 4.0, LABEL_W, 17.0));
        l.setHidden(true);
        root.addSubview(&l);

        let input = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            top_rect(H, INPUT_X, top, INPUT_W, INPUT_H),
        );
        input.setBezeled(true);
        input.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
        input.setFocusRingType(NSFocusRingType::Default);
        input.setFont(Some(&NSFont::systemFontOfSize(14.0)));
        input.setUsesSingleLineMode(true);
        // SAFETY: `Ui` keeps the delegate alive as long as the field (the field holds it weakly).
        unsafe { input.setDelegate(Some(ProtocolObject::from_ref(delegate))) };
        input.setHidden(true);
        root.addSubview(&input);
        labels.push(l);
        inputs.push(input);
    }

    let error = label(mtm, 12.0, &NSColor::systemRedColor());
    error.setFrame(top_rect(H, INPUT_X, FIELDS_TOP, INPUT_W, ERROR_H));
    error.setHidden(true);
    root.addSubview(&error);

    let multiline = (0..FORM_FIELDS).map(|_| Cell::new(false)).collect();
    FormViews { title, labels, inputs, multiline, error, mode: Cell::new(Mode::Search) }
}

/// The index of the form field that is `object`, if it is one.
pub(super) fn field_index(object: *const AnyObject) -> Option<usize> {
    with_ui(|ui| {
        ui.form.inputs.iter().position(|f| std::ptr::eq(Retained::as_ptr(f).cast(), object))
    })
    .flatten()
}

/// Return in a multiline form field: insert a newline into `view` instead of submitting.
/// True when it did.
pub(super) fn newline(object: *const AnyObject, view: &NSTextView, sel: Sel) -> bool {
    let multiline = sel == sel!(insertNewline:)
        && field_index(object)
            .and_then(|i| with_ui(|ui| ui.form.multiline.get(i).is_some_and(Cell::get)))
            .unwrap_or(false);
    if multiline {
        // SAFETY: NSTextView implements this action; nil is a valid sender.
        unsafe { view.insertNewlineIgnoringFieldEditor(None) };
    }
    multiline
}

/// The top row of each field that fits, and the rows they take: a multiline field takes
/// `MULTILINE_ROWS`.
fn layout(multiline: impl IntoIterator<Item = bool>) -> (Vec<usize>, usize) {
    let mut tops = vec![];
    let mut row = 0;
    for m in multiline {
        let rows = if m { MULTILINE_ROWS } else { 1 };
        if row + rows > FORM_FIELDS {
            break;
        }
        tops.push(row);
        row += rows;
    }
    (tops, row)
}

/// Make `input` a one-line field or a taller wrapping one with its top at row `row`.
fn shape(input: &NSTextField, row: usize, multiline: bool) {
    let rows = if multiline { MULTILINE_ROWS } else { 1 };
    let h = INPUT_H + (rows - 1) as f64 * FIELD_ROW_H;
    input.setFrame(top_rect(H, INPUT_X, FIELDS_TOP + row as f64 * FIELD_ROW_H, INPUT_W, h));
    input.setUsesSingleLineMode(!multiline);
    input.setLineBreakMode(if multiline {
        NSLineBreakMode::ByWordWrapping
    } else {
        NSLineBreakMode::ByClipping
    });
    if let Some(cell) = input.cell() {
        cell.setWraps(multiline);
        cell.setScrollable(!multiline);
    }
}

/// The view that should have keyboard focus in `mode`; `None` means the panel itself.
pub(super) fn responder(ui: &super::Ui, mode: Mode) -> Option<&NSResponder> {
    match mode {
        Mode::Search => Some(&ui.field),
        Mode::Title => None,
        Mode::Form(i) => ui.form.inputs.get(i).map(|f| &**f as &NSResponder),
    }
}

/// Switch the search area to `mode`, moving focus only when the mode changes, so a render
/// that keeps the mode leaves the caret and selection alone.
fn enter(ui: &super::Ui, mode: Mode) {
    let form = matches!(mode, Mode::Form(_));
    ui.field.setHidden(mode != Mode::Search);
    ui.form.title.setHidden(mode == Mode::Search);
    ui.form.error.setHidden(!form);
    if !form {
        for (l, f) in ui.form.labels.iter().zip(&ui.form.inputs) {
            l.setHidden(true);
            f.setHidden(true);
        }
    }
    ui.empty.setHidden(form);
    let refocus = match mode {
        Mode::Form(i) => ui.form.inputs.get(i).is_some_and(|f| f.currentEditor().is_none()),
        _ => ui.form.mode.get() != mode,
    };
    ui.form.mode.set(mode);
    if refocus {
        ui.panel.makeFirstResponder(responder(ui, mode));
    }
}

/// Set up the search area for a list: the search field, or a read-only `title`.
pub(super) fn list_mode(ui: &super::Ui, title: Option<&str>) {
    if let Some(title) = title {
        ui.form.title.setStringValue(&ns(title));
    }
    enter(ui, if title.is_some() { Mode::Title } else { Mode::Search });
}

/// Draw a form. A field's text is only replaced when it differs, so typing keeps its caret.
pub fn render_form(frame: &FormFrame) {
    with_ui(|ui| {
        for row in &ui.rows {
            row.view.setHidden(true);
        }
        ui.text.setHidden(true);
        ui.form.title.setStringValue(&ns(frame.title));
        let (tops, rows) = layout(frame.fields.iter().map(|f| f.multiline));
        let shown = tops.len();
        let fields = frame.fields.iter().zip(&tops).map(Some).chain(std::iter::repeat(None));
        let views = ui.form.labels.iter().zip(&ui.form.inputs).zip(&ui.form.multiline);
        for (((l, input), multiline), field) in views.zip(fields) {
            l.setHidden(field.is_none());
            input.setHidden(field.is_none());
            let Some((field, &row)) = field else { continue };
            l.setFrame(top_rect(
                H,
                20.0,
                FIELDS_TOP + row as f64 * FIELD_ROW_H + 4.0,
                LABEL_W,
                17.0,
            ));
            l.setStringValue(&ns(field.label));
            shape(input, row, field.multiline);
            multiline.set(field.multiline);
            if input.stringValue().to_string() != field.value {
                input.setStringValue(&ns(field.value));
            }
            input.setPlaceholderString(Some(&ns(field.placeholder)));
        }
        // The error line sits right under the last field.
        let error_top = FIELDS_TOP + rows as f64 * FIELD_ROW_H - (FIELD_ROW_H - INPUT_H - 8.0);
        ui.form.error.setFrame(top_rect(H, INPUT_X, error_top, INPUT_W, ERROR_H));
        ui.form.error.setStringValue(&ns(frame.error));
        ui.footer_left.setStringValue(&ns(frame.footer));
        ui.footer_action.setStringValue(&ns(frame.action));
        enter(ui, if shown == 0 { Mode::Title } else { Mode::Form(frame.focused.min(shown - 1)) });
    });
}

/// The text in form field `index`, as typed so far.
pub fn field_value(index: usize) -> String {
    with_ui(|ui| ui.form.inputs.get(index).map(|f| f.stringValue().to_string()))
        .flatten()
        .unwrap_or_default()
}

/// The form field being edited, which a mouse click can change behind the controller's back.
pub fn focused_field() -> Option<usize> {
    with_ui(|ui| ui.form.inputs.iter().position(|f| f.currentEditor().is_some())).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_fields_take_more_rows_and_overflow_is_dropped() {
        assert_eq!(layout([false, false]), (vec![0, 1], 2));
        assert_eq!(layout([false, true, false]), (vec![0, 1, 4], 5));
        assert_eq!(layout([true, true, true]), (vec![0, 3], 6));
        assert_eq!(layout([false; 8]), (vec![0, 1, 2, 3, 4, 5], 6));
    }
}
