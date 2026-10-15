//! Secondary actions on an item, listed by cmd+K (`Module::actions`, run by `Module::act`).

use std::borrow::Cow;

use super::Icon;

/// One entry of an item's action menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// The module's name for the action; `Module::act` gets it back.
    pub key: Cow<'static, str>,
    pub title: String,
    pub icon: Icon,
    /// Shown at the row's end, e.g. "⌘E". Empty for none. The controller binds only "⌘Q":
    /// on a list it runs the selected item's action with that hint. Others are display only.
    pub shortcut_hint: &'static str,
}

impl Action {
    /// An action with no shortcut hint.
    pub fn new(key: impl Into<Cow<'static, str>>, title: impl Into<String>, icon: Icon) -> Action {
        Action { key: key.into(), title: title.into(), icon, shortcut_hint: "" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_action_has_no_hint() {
        let a = Action::new("edit", "Edit Quicklink", Icon::symbol("pencil"));
        assert_eq!((&*a.key, a.title.as_str(), a.shortcut_hint), ("edit", "Edit Quicklink", ""));
        assert_eq!(a.icon, Icon::symbol("pencil"));
        let owned = Action::new(format!("run/{}", 2), "Run", Icon::symbol("play"));
        assert_eq!(
            (owned.key, Action::new("edit", "", Icon::symbol("")).key),
            ("run/2".into(), Cow::Borrowed("edit"))
        );
    }
}
