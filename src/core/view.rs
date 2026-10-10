//! What a module's activation does, and the lists modules show in place of root search.

use super::{Confirm, Item};

/// The result of activating an item.
#[derive(Debug)]
pub enum Outcome {
    /// Close the launcher.
    Hide,
    /// Keep the launcher as it is; `Some` shows a status line in the footer.
    Stay(Option<String>),
    /// Show a module's list. The registry asks the owning module (`ListView::module`) to
    /// open it, so any module may push any other module's view by name.
    Push(ListView),
    /// Show a module's form. Like `Push`, a request by name: the registry asks `module` to
    /// build form `name`, so any module may open any other module's form.
    Form { module: &'static str, name: String },
    /// Ask the user to confirm; on confirm the registry hands `token` back to the owning
    /// module (`Confirm::module`).
    Confirm(Confirm),
    /// Load config.toml again, rebind hotkeys, then back to root search with the result.
    ReloadConfig,
}

/// A module-owned list that replaces root search, e.g. clipboard history.
#[derive(Clone, Debug)]
pub struct ListView {
    /// The module that owns the view and fills it.
    pub module: &'static str,
    /// The module's name for the view.
    pub name: String,
    pub placeholder: String,
    /// A read-only title in place of the search field. The view then takes no typing, so the
    /// vim keys work: J/K, ⌃D/⌃U and G/⇧G scroll `text` (or move the selection when there is
    /// no text); Up/Down and ⌃J/⌃K still move the selection.
    pub title: Option<String>,
    /// Footer text when no status is showing.
    pub footer: String,
    /// Shown when `items` is empty.
    pub empty: String,
    pub items: Vec<Item>,
    /// Read-only text under the items, wrapped, in a fixed-width font (e.g. an agent's
    /// reply). Empty: none. It scrolls (mouse, and the vim keys with a `title`); new text
    /// starts at its top.
    pub text: String,
    /// New `text` starts at its end, and stays there as it grows while the user is at the end
    /// (output, a log).
    pub text_tail: bool,
    /// Activating an item counts toward its frecency.
    pub record_use: bool,
    /// Escape hides the launcher instead of going back to root search.
    pub escape_hides: bool,
}

impl ListView {
    /// An empty view: a search field, no items, "No Results" when empty, no frecency, Escape
    /// goes back.
    pub fn new(module: &'static str, name: impl Into<String>) -> ListView {
        ListView {
            module,
            name: name.into(),
            placeholder: String::new(),
            title: None,
            footer: String::new(),
            empty: "No Results".into(),
            items: vec![],
            text: String::new(),
            text_tail: false,
            record_use: false,
            escape_hides: false,
        }
    }

    /// Same module and name.
    pub fn is(&self, module: &str, name: &str) -> bool {
        self.module == module && self.name == name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_view_defaults() {
        let v = ListView::new("clip", "history");
        assert!(v.is("clip", "history"));
        assert!(!v.is("clip", "other") && !v.is("switcher", "history"));
        assert_eq!(v.empty, "No Results");
        assert!(v.items.is_empty() && v.text.is_empty() && !v.record_use && !v.escape_hides);
        assert!(v.title.is_none() && !v.text_tail);
    }
}
