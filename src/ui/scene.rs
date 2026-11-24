//! The scene: a plain-text outline of what the launcher shows, built from the last `View` or
//! `Form` handed to the panel, so a driven instance can be read without a screenshot. Pure;
//! `ui` keeps the last outline.

use std::fmt::Write as _;

use super::View;
use crate::core::{Field, Form, Item, Tone};

/// The outline of a list screen: its `kind`, the title (or the search `query` when it has
/// none), the rows the panel shows from `view.scroll` with the selected one marked `>`, the
/// empty text when there are no items, then footer, action and the text block.
pub fn list(view: &View, query: &str, visible: usize) -> String {
    let mut out = format!("screen: {}\n", view.screen);
    match view.title {
        Some(title) => line(&mut out, "title", title),
        None => line(&mut out, "query", query),
    }
    let total = view.items.len();
    if total == 0 {
        line(&mut out, "empty", view.empty);
    } else {
        let first = view.scroll.min(total - 1);
        let last = (first + visible).min(total);
        let _ = writeln!(out, "rows: {}-{last} of {total}", first + 1);
        for (index, item) in view.items.iter().enumerate().take(last).skip(first) {
            row(&mut out, index, item, view.selected == Some(index));
        }
    }
    line(&mut out, "footer", view.footer);
    line(&mut out, "action", view.action);
    if !view.text.is_empty() {
        out.push_str(if view.text_tail { "text (tail):\n" } else { "text:\n" });
        for text in view.text.lines() {
            let _ = writeln!(out, "  {text}");
        }
    }
    out
}

/// The outline of `form`: its title, one line per field with the focused one marked `>`,
/// the error, then `footer` and the submit hint.
pub fn form(form: &Form, footer: &str) -> String {
    let mut out = String::from("screen: form\n");
    line(&mut out, "title", &form.title);
    out.push_str("fields:\n");
    for (index, f) in form.fields.iter().enumerate() {
        field(&mut out, f, index == form.focused);
    }
    if let Some(error) = &form.error {
        line(&mut out, "error", error);
    }
    line(&mut out, "footer", footer);
    line(&mut out, "action", &form.submit_hint());
    out
}

fn line(out: &mut String, key: &str, value: &str) {
    let _ = writeln!(out, "{key}: {value}");
}

/// `> 3 Title — subtitle  [accessory] (ok)`: the 1-based index among all items.
fn row(out: &mut String, index: usize, item: &Item, selected: bool) {
    let mark = if selected { '>' } else { ' ' };
    let _ = write!(out, "{mark} {} {}", index + 1, item.title);
    if !item.subtitle.is_empty() {
        let _ = write!(out, " — {}", item.subtitle);
    }
    if !item.accessory.is_empty() {
        let _ = write!(out, "  [{}]", item.accessory);
    }
    let tone = match item.tone {
        Tone::Neutral => "",
        Tone::Ok => " (ok)",
        Tone::Warn => " (warn)",
        Tone::Error => " (error)",
    };
    out.push_str(tone);
    out.push('\n');
}

/// `> Label: "value"`, quoted so a multiline value stays on one line; the placeholder shows
/// while the value is empty.
fn field(out: &mut String, f: &Field, focused: bool) {
    let mark = if focused { '>' } else { ' ' };
    let _ = write!(out, "{mark} {}: {:?}", f.label, f.value);
    if f.value.is_empty() && !f.placeholder.is_empty() {
        let _ = write!(out, " (placeholder {:?})", f.placeholder);
    }
    if f.required {
        out.push_str(" (required)");
    }
    if f.multiline {
        out.push_str(" (multiline)");
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Icon, ItemId};

    fn item(title: &str) -> Item {
        Item::new(ItemId::new("test", title), title, "Open", Icon::symbol("star"))
    }

    fn view(items: &[Item]) -> View<'_> {
        View {
            screen: "root",
            title: None,
            items,
            selected: Some(0),
            scroll: 0,
            footer: "Flick",
            empty: "No Results",
            action: "Open  ↵",
            text: "",
            text_tail: false,
        }
    }

    #[test]
    fn a_list_names_its_query_rows_selection_footer_and_action() {
        let mut items = vec![item("Safari"), item("Slack")];
        items[0].subtitle = "Application".into();
        items[0].accessory = "⌘1".into();
        items[1].tone = Tone::Warn;
        let mut v = view(&items);
        v.selected = Some(1);
        assert_eq!(
            list(&v, "sa", 8),
            "screen: root\nquery: sa\nrows: 1-2 of 2\n  1 Safari — Application  [⌘1]\n\
             > 2 Slack (warn)\nfooter: Flick\naction: Open  ↵\n"
        );
    }

    #[test]
    fn a_title_replaces_the_query_and_only_the_visible_rows_show() {
        let mut items: Vec<Item> = ["a", "b", "c", "d"].map(item).into();
        items[1].tone = Tone::Ok;
        items[2].tone = Tone::Error;
        let mut v = view(&items);
        v.screen = "list";
        v.title = Some("Tasks");
        v.scroll = 1;
        v.selected = Some(2);
        assert_eq!(
            list(&v, "ignored", 2),
            "screen: list\ntitle: Tasks\nrows: 2-3 of 4\n  2 b (ok)\n> 3 c (error)\n\
             footer: Flick\naction: Open  ↵\n"
        );
    }

    #[test]
    fn no_items_show_the_empty_text_and_the_text_block_follows() {
        let mut v = view(&[]);
        v.screen = "confirm";
        v.selected = None;
        v.text = "one\ntwo";
        assert_eq!(
            list(&v, "", 8),
            "screen: confirm\nquery: \nempty: No Results\nfooter: Flick\naction: Open  ↵\n\
             text:\n  one\n  two\n"
        );
        v.text_tail = true;
        assert!(list(&v, "", 8).ends_with("text (tail):\n  one\n  two\n"));
    }

    #[test]
    fn a_form_marks_focus_and_shows_values_placeholders_and_the_error() {
        let mut f = Form::new("test", "edit", "New Link");
        f.fields.push(Field::new("name", "Name").value("Docs").required());
        f.fields.push(Field::new("url", "URL").placeholder("https://"));
        f.fields.push(Field::new("note", "Note").value("a\nb").multiline());
        f.focused = 1;
        f.submit_label = "Save".into();
        f.error = Some("URL is required".into());
        assert_eq!(
            form(&f, "⇥  Next Field"),
            "screen: form\ntitle: New Link\nfields:\n  Name: \"Docs\" (required)\n\
             > URL: \"\" (placeholder \"https://\")\n  Note: \"a\\nb\" (multiline)\n\
             error: URL is required\nfooter: ⇥  Next Field\naction: Save  ⌘↵\n"
        );
    }
}
