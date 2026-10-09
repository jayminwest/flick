//! Secondary actions on an item, listed by cmd+K (`Module::actions`, run by `Module::act`).

use super::Icon;

/// One entry of an item's action menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// The module's name for the action; `Module::act` gets it back.
    pub key: &'static str,
    pub title: String,
    pub icon: Icon,
    /// Shown at the row's end, e.g. "⌘E". Empty for none. Display only: the controller
    /// does not bind it.
    pub shortcut_hint: &'static str,
}

impl Action {
    /// An action with no shortcut hint.
    pub fn new(key: &'static str, title: impl Into<String>, icon: Icon) -> Action {
        Action { key, title: title.into(), icon, shortcut_hint: "" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_action_has_no_hint() {
        let a = Action::new("edit", "Edit Quicklink", Icon::Symbol("pencil"));
        assert_eq!((a.key, a.title.as_str(), a.shortcut_hint), ("edit", "Edit Quicklink", ""));
        assert_eq!(a.icon, Icon::Symbol("pencil"));
    }
}
