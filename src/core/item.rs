//! Result items and the ids that route them to their module.

use std::fmt;
use std::ops::Deref;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    File(PathBuf),
    Symbol(&'static str),
}

/// Which module owns an item, and the module's key for it. Serializes as `<module>:<key>`;
/// that string keys the `usage` table, so a module must never change how it builds keys.
///
/// `arg` rides along to the module on activation (e.g. a quicklink's typed query) but is
/// not part of the serialized id, so it never splits an item's usage history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemId {
    module: &'static str,
    /// `<module>:<key>`.
    id: String,
    arg: Option<String>,
}

impl ItemId {
    pub fn new(module: &'static str, key: impl fmt::Display) -> ItemId {
        ItemId { module, id: format!("{module}:{key}"), arg: None }
    }

    #[must_use]
    pub fn with_arg(mut self, arg: impl Into<String>) -> ItemId {
        self.arg = Some(arg.into());
        self
    }

    pub fn module(&self) -> &'static str {
        self.module
    }

    pub fn key(&self) -> &str {
        &self.id[self.module.len() + 1..]
    }

    pub fn arg(&self) -> Option<&str> {
        self.arg.as_deref()
    }

    /// The serialized id, `<module>:<key>`.
    pub fn as_str(&self) -> &str {
        &self.id
    }
}

impl Deref for ItemId {
    type Target = str;

    fn deref(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id)
    }
}

impl PartialEq<str> for ItemId {
    fn eq(&self, other: &str) -> bool {
        self.id == other
    }
}

impl PartialEq<&str> for ItemId {
    fn eq(&self, other: &&str) -> bool {
        self.id == *other
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: ItemId,
    pub title: String,
    pub subtitle: String,
    pub accessory: String,
    pub icon: Icon,
    /// Extra text the fuzzy matcher searches besides the title.
    pub keywords: Vec<String>,
    /// What Enter does, shown in the footer: "Open Application", "Paste", ...
    pub verb: &'static str,
    /// What Tab does on a list.
    pub tab: Tab,
    /// The status the row's icon shows: a symbol icon is tinted green, orange or red.
    pub tone: Tone,
}

/// A status color for an item, drawn as a tint on its SF Symbol icon (system green, orange
/// or red, which follow light and dark mode). Only for state that is plainly good or bad;
/// most items stay `Neutral`. A file icon (an app's) ignores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    /// The usual secondary-label tint.
    #[default]
    Neutral,
    /// Green: running, done, healthy.
    Ok,
    /// Orange: needs a look (stale, warning, blocked).
    Warn,
    /// Red: failed, down.
    Error,
}

/// What Tab does to a selected item in root search or a module list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    /// Nothing.
    #[default]
    None,
    /// The same as Enter (a quicklink that still needs its argument).
    Activate,
    /// Run the item's action with this key (`Module::act`), as if picked in the ⌘K menu.
    Act(&'static str),
}

impl Item {
    /// An item with no subtitle, accessory or keywords.
    pub fn new(id: ItemId, title: impl Into<String>, verb: &'static str, icon: Icon) -> Item {
        Item {
            id,
            title: title.into(),
            subtitle: String::new(),
            accessory: String::new(),
            icon,
            keywords: vec![],
            verb,
            tab: Tab::None,
            tone: Tone::Neutral,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_serializes_as_module_colon_key() {
        let id = ItemId::new("app", "/Applications/Safari.app");
        assert_eq!(id.as_str(), "app:/Applications/Safari.app");
        assert_eq!(id.to_string(), "app:/Applications/Safari.app");
        assert_eq!(id.module(), "app");
        assert_eq!(id.key(), "/Applications/Safari.app");
        assert_eq!(id.arg(), None);
        assert_eq!(id, "app:/Applications/Safari.app");
        assert_eq!(id, *"app:/Applications/Safari.app");
        assert_eq!(id.split(':').next(), Some("app"));
    }

    #[test]
    fn keys_may_hold_colons() {
        let id = ItemId::new("quicklink", "a:b");
        assert_eq!((id.module(), id.key()), ("quicklink", "a:b"));
    }

    #[test]
    fn arg_is_not_part_of_the_id() {
        let id = ItemId::new("quicklink", "Google").with_arg("rust");
        assert_eq!(id.as_str(), "quicklink:Google");
        assert_eq!(id.arg(), Some("rust"));
        assert_ne!(id, ItemId::new("quicklink", "Google"));
    }

    #[test]
    fn new_item_has_empty_extras() {
        let item = Item::new(
            ItemId::new("builtin", "Quit Flick"),
            "Quit Flick",
            "Run Command",
            Icon::Symbol("power"),
        );
        assert_eq!(item.title, "Quit Flick");
        assert!(item.subtitle.is_empty() && item.accessory.is_empty() && item.keywords.is_empty());
        assert_eq!(item.tab, Tab::None);
        assert_eq!((item.tone, Tone::default()), (Tone::Neutral, Tone::Neutral));
        assert_eq!(Tab::default(), Tab::None);
        assert_eq!(item.verb, "Run Command");
        assert_eq!(item.icon, Icon::Symbol("power"));
    }
}
