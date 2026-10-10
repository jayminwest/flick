//! The launcher view: turns results and forms into what the platform panel draws, and wires
//! the panel's typing and keys to the controller.

use crate::core::{Field, Form, Icon, Item, Tone};
use crate::platform::panel::{self, FormField, FormFrame, Frame, Handlers, Row};

pub use crate::platform::panel::{
    VISIBLE_ROWS, field_value, focused_field, hide, is_visible, place, query, scroll_text,
    set_opacity, set_query, show, snapshot,
};

pub fn init() {
    panel::init(Handlers {
        query_changed: crate::app::query_changed,
        field_changed: crate::app::field_changed,
        key: crate::app::command,
    });
}

pub struct View<'a> {
    /// A read-only title in place of the search field.
    pub title: Option<&'a str>,
    pub items: &'a [Item],
    /// The highlighted item; `None` for read-only rows.
    pub selected: Option<usize>,
    pub scroll: usize,
    pub footer: &'a str,
    pub empty: &'a str,
    /// Right-aligned footer text: what Return does.
    pub action: &'a str,
    /// Read-only text under the rows (`ListView::text`).
    pub text: &'a str,
    /// `ListView::text_tail`.
    pub text_tail: bool,
}

pub fn render(view: &View) {
    let rows: Vec<Row> = view
        .items
        .iter()
        .skip(view.scroll)
        .take(VISIBLE_ROWS)
        .map(|item| Row {
            title: &item.title,
            subtitle: &item.subtitle,
            accessory: &item.accessory,
            icon: match &item.icon {
                Icon::File(p) => panel::Icon::File(p),
                Icon::Symbol(s) => panel::Icon::Symbol(s),
            },
            tone: match item.tone {
                Tone::Neutral => panel::Tone::Neutral,
                Tone::Ok => panel::Tone::Ok,
                Tone::Warn => panel::Tone::Warn,
                Tone::Error => panel::Tone::Error,
            },
        })
        .collect();
    panel::render(&Frame {
        title: view.title,
        rows: &rows,
        selected: view.selected.and_then(|s| s.checked_sub(view.scroll)),
        empty: if view.items.is_empty() { view.empty } else { "" },
        footer: view.footer,
        action: view.action,
        text: view.text,
        text_tail: view.text_tail,
    });
}

/// Draw `form`, with `footer` on the left and its submit label as the action.
pub fn render_form(form: &Form, footer: &str) {
    let fields: Vec<FormField> = form.fields.iter().map(form_field).collect();
    panel::render_form(&FormFrame {
        title: &form.title,
        fields: &fields,
        focused: form.focused,
        error: form.error.as_deref().unwrap_or(""),
        footer,
        action: &form.submit_hint(),
    });
}

fn form_field(f: &Field) -> FormField<'_> {
    FormField {
        label: &f.label,
        value: &f.value,
        placeholder: &f.placeholder,
        multiline: f.multiline,
    }
}
