//! Confirmation screens: a module asks before it runs an action (`Outcome::Confirm`), and
//! hears back through `Module::confirmed` with its own token.

/// A read-only row of a confirmation screen, e.g. a file an uninstall would remove.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmRow {
    pub title: String,
    pub subtitle: String,
    pub accessory: String,
}

impl ConfirmRow {
    /// A row with no subtitle or accessory.
    #[cfg_attr(not(test), expect(dead_code, reason = "modules call it from flick-37d5 on"))]
    pub fn new(title: impl Into<String>) -> ConfirmRow {
        ConfirmRow { title: title.into(), subtitle: String::new(), accessory: String::new() }
    }
}

/// A question the owning module asks before an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    /// The module that asked; it gets `token` back when the user confirms.
    pub module: &'static str,
    /// The module's note of what to do on confirm, e.g. "delete/Docs".
    pub token: String,
    pub title: String,
    pub rows: Vec<ConfirmRow>,
    /// What confirming does, e.g. "Delete Quicklink".
    pub label: String,
    /// Plain Enter does not confirm; only cmd+Enter does.
    pub destructive: bool,
}

impl Confirm {
    /// A non-destructive confirmation with no rows, "Confirm" as its label.
    #[cfg_attr(not(test), expect(dead_code, reason = "modules call it from flick-37d5 on"))]
    pub fn new(
        module: &'static str,
        token: impl Into<String>,
        title: impl Into<String>,
    ) -> Confirm {
        Confirm {
            module,
            token: token.into(),
            title: title.into(),
            rows: vec![],
            label: "Confirm".into(),
            destructive: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_confirm_and_row_defaults() {
        let c = Confirm::new("quicklink", "delete/Docs", "Delete Docs?");
        assert_eq!(
            (c.module, c.token.as_str(), c.title.as_str()),
            ("quicklink", "delete/Docs", "Delete Docs?")
        );
        assert_eq!(c.label, "Confirm");
        assert!(c.rows.is_empty() && !c.destructive);
        let r = ConfirmRow::new("a");
        assert_eq!(r.title, "a");
        assert!(r.subtitle.is_empty() && r.accessory.is_empty());
    }
}
