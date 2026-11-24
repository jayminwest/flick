//! The launcher view: turns results and forms into what the platform panel draws, and wires
//! the panel's typing and keys to the controller.

mod scene;

use std::cell::RefCell;

use crate::core::{Field, Form, Icon, Item, Tone};
use crate::platform::panel::{self, FormField, FormFrame, Frame, Handlers, Row};

pub use crate::platform::panel::{
    VISIBLE_ROWS, field_value, focused_field, hide, is_visible, place, query, scroll_text,
    set_opacity, set_placeholder, set_query, show, snapshot,
};

pub fn init() {
    panel::init(Handlers {
        query_changed: crate::app::query_changed,
        field_changed: crate::app::field_changed,
        key: crate::app::command,
    });
}

pub struct View<'a> {
    /// What kind of screen this is, for the scene: `root`, `list`, `actions` or `confirm`.
    pub screen: &'static str,
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
    keep(scene::list(view, &query(), VISIBLE_ROWS));
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
    keep(scene::form(form, footer));
}

thread_local! {
    /// The outline of the last view or form handed to the panel.
    static SCENE: RefCell<String> = const { RefCell::new(String::new()) };
}

fn keep(outline: String) {
    SCENE.set(outline);
}

/// What the panel shows, as a text outline of the last view or form it was handed (see
/// `scene`); empty before the first render.
#[cfg_attr(not(test), expect(dead_code, reason = "the drive verbs (flick-355ef9) read it"))]
pub fn scene() -> String {
    SCENE.with_borrow(Clone::clone)
}

fn form_field(f: &Field) -> FormField<'_> {
    FormField {
        label: &f.label,
        value: &f.value,
        placeholder: &f.placeholder,
        multiline: f.multiline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ItemId;

    #[test]
    fn the_last_view_or_form_rendered_is_the_scene() {
        let items = [Item::new(ItemId::new("test", "a"), "Alpha", "Open", Icon::symbol("star"))];
        render(&View {
            screen: "list",
            title: Some("Things"),
            items: &items,
            selected: Some(0),
            scroll: 0,
            footer: "3 things",
            empty: "",
            action: "Open  ↵",
            text: "",
            text_tail: false,
        });
        assert!(scene().starts_with("screen: list\ntitle: Things\nrows: 1-1 of 1\n> 1 Alpha\n"));
        let mut form = Form::new("test", "edit", "Edit");
        form.fields.push(Field::new("name", "Name"));
        render_form(&form, "⇥  Next Field");
        assert!(scene().starts_with("screen: form\ntitle: Edit\nfields:\n> Name: \"\"\n"));
    }
}
