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
    /// Footer text when no status is showing.
    pub footer: String,
    /// Shown when `items` is empty.
    pub empty: String,
    pub items: Vec<Item>,
    /// Activating an item counts toward its frecency.
    pub record_use: bool,
    /// Escape hides the launcher instead of going back to root search.
    pub escape_hides: bool,
}

impl ListView {
    /// An empty view: no items, "No Results" when empty, no frecency, Escape goes back.
    pub fn new(module: &'static str, name: impl Into<String>) -> ListView {
        ListView {
            module,
            name: name.into(),
            placeholder: String::new(),
            footer: String::new(),
            empty: "No Results".into(),
            items: vec![],
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
        assert!(v.items.is_empty() && !v.record_use && !v.escape_hides);
    }
}
