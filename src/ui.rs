//! The launcher view: turns results into what the platform panel draws, and wires the panel's
//! typing and keys to the controller.

use crate::platform::panel::{self, Frame, Handlers, Row};
use crate::search::{Icon, Item};

pub use crate::platform::panel::{
    VISIBLE_ROWS, hide, is_visible, query, set_query, show, snapshot,
};

pub fn init() {
    panel::init(Handlers { query_changed: crate::app::query_changed, key: crate::app::command });
}

pub struct View<'a> {
    pub items: &'a [Item],
    pub selected: usize,
    pub scroll: usize,
    pub footer: &'a str,
    pub empty: &'a str,
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
        })
        .collect();
    let action = view.items.get(view.selected).map(|i| format!("{}  ↵", i.action.verb()));
    panel::render(&Frame {
        rows: &rows,
        selected: view.selected.checked_sub(view.scroll),
        empty: if view.items.is_empty() { view.empty } else { "" },
        footer: view.footer,
        action: action.as_deref().unwrap_or(""),
    });
}
