//! What the launcher shows, and the pure decisions behind its form, action-menu and
//! confirmation screens. No `AppKit` calls, so it unit-tests.

use crate::core::{Action, Confirm, ConfirmRow, Form, Icon, Item, ItemId, ListView, Ranker};
use crate::platform::panel::Key;

/// The screen on the panel.
pub enum Screen {
    /// Root search.
    Root,
    /// A module's list.
    List(ListView),
    /// A module's form.
    Form(Form),
    /// The action menu (⌘K) of `target`, an item of the `back` screen.
    Actions { target: Item, actions: Vec<Action>, back: Back },
    /// A module's question; cancelling returns to `back`.
    Confirm { confirm: Confirm, back: Back },
}

/// The screen an action menu or a confirmation returns to, as the user left it.
pub struct Back {
    pub screen: Box<Screen>,
    pub query: String,
    pub selected: usize,
    pub scroll: usize,
}

impl Screen {
    /// Root search, or `view` when it is the one showing.
    pub fn shows(&self, view: Option<&ListView>) -> bool {
        match (self, view) {
            (Screen::Root, None) => true,
            (Screen::List(v), Some(r)) => v.is(r.module, &r.name),
            _ => false,
        }
    }

    /// The module view this screen holds: the list on it, or the one an action menu or a
    /// confirmation returns to (Escape restores it without asking its module again).
    pub fn view(&self) -> Option<&ListView> {
        match self {
            Screen::List(view) => Some(view),
            Screen::Actions { back, .. } | Screen::Confirm { back, .. } => back.screen.view(),
            Screen::Root | Screen::Form(_) => None,
        }
    }

    /// Root search or a module list: the screens whose items have actions.
    pub fn is_list(&self) -> bool {
        matches!(self, Screen::Root | Screen::List(_))
    }
}

/// A module view by module and name.
pub type Held = (&'static str, String);

/// Record that `next` goes on the panel: `held` becomes the view it holds. Returns the view
/// `held` named before when `next` no longer holds it, for its module's `closed`.
pub fn hold(held: &mut Option<Held>, next: &Screen) -> Option<Held> {
    let now = next.view().map(|v| (v.module, v.name.clone()));
    std::mem::replace(held, now).filter(|before| held.as_ref() != Some(before))
}

/// The action menu's rows: in the module's order for an empty `query`, else ranked.
pub fn action_items(ranker: &mut Ranker, query: &str, actions: &[Action]) -> Vec<Item> {
    let items = actions.iter().map(|a| {
        let mut item =
            Item::new(ItemId::new("action", a.key), &a.title, "Run Action", a.icon.clone());
        item.accessory = a.shortcut_hint.into();
        item
    });
    if query.trim().is_empty() {
        items.collect()
    } else {
        ranker.rank(query, items.collect(), |_| 0.0)
    }
}

/// A confirmation's read-only rows, as items.
pub fn confirm_items(confirm: &Confirm) -> Vec<Item> {
    confirm.rows.iter().enumerate().map(|(i, row)| confirm_item(i, row)).collect()
}

fn confirm_item(index: usize, row: &ConfirmRow) -> Item {
    let icon = Icon::Symbol("smallcircle.filled.circle");
    Item {
        subtitle: row.subtitle.clone(),
        accessory: row.accessory.clone(),
        ..Item::new(ItemId::new("confirm", index), &row.title, "", icon)
    }
}

/// The footer's right side for a list: what Enter does to `item`, and ⌘K when it has
/// actions. Empty with no item.
pub fn list_hint(item: Option<&Item>, has_actions: bool) -> String {
    match item {
        Some(i) if has_actions => format!("{}  ↵    Actions  ⌘K", i.verb),
        Some(i) => format!("{}  ↵", i.verb),
        None => String::new(),
    }
}

/// The footer's right side for a confirmation: its label and the key that confirms.
pub fn confirm_hint(confirm: &Confirm) -> String {
    format!("{}  {}", confirm.label, if confirm.destructive { "⌘↵" } else { "↵" })
}

/// Why `form` cannot be submitted yet: its first blank required field.
pub fn missing(form: &Form) -> Option<String> {
    form.missing_required().first().map(|f| format!("{} is required", f.label))
}

/// What a key does on a confirmation screen.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfirmStep {
    Confirm,
    /// Plain Enter on a destructive confirmation: say that ⌘↵ confirms.
    Hint,
    Cancel,
    Scroll(isize),
    Ignore,
}

/// Rows that ⌃D and ⌃U scroll: half the visible rows.
const HALF_PAGE: isize = (crate::ui::VISIBLE_ROWS / 2) as isize;

pub fn confirm_step(destructive: bool, key: Key) -> ConfirmStep {
    match key {
        Key::Enter if destructive => ConfirmStep::Hint,
        Key::Enter | Key::CmdEnter => ConfirmStep::Confirm,
        Key::Escape => ConfirmStep::Cancel,
        Key::Up => ConfirmStep::Scroll(-1),
        Key::Down => ConfirmStep::Scroll(1),
        Key::PageUp => ConfirmStep::Scroll(-HALF_PAGE),
        Key::PageDown => ConfirmStep::Scroll(HALF_PAGE),
        Key::Top => ConfirmStep::Scroll(isize::MIN),
        Key::Bottom => ConfirmStep::Scroll(isize::MAX),
        _ => ConfirmStep::Ignore,
    }
}

/// The first visible row after scrolling `scroll` by `delta` through `rows` rows, `visible`
/// at a time.
pub fn scrolled(scroll: usize, delta: isize, rows: usize, visible: usize) -> usize {
    scroll.saturating_add_signed(delta).min(rows.saturating_sub(visible))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Field;

    fn actions() -> Vec<Action> {
        vec![
            Action::new("open", "Open", Icon::Symbol("arrow.up.right")),
            Action {
                shortcut_hint: "⌘E",
                ..Action::new("edit", "Edit Quicklink", Icon::Symbol("pencil"))
            },
            Action::new("delete", "Delete Quicklink", Icon::Symbol("trash")),
        ]
    }

    #[test]
    fn action_rows_keep_module_order_until_typed_in() {
        let mut ranker = Ranker::new();
        let titles = |items: Vec<Item>| items.into_iter().map(|i| i.title).collect::<Vec<_>>();
        let all = action_items(&mut ranker, " ", &actions());
        assert_eq!(all[1].id.key(), "edit");
        assert_eq!(all[1].accessory, "⌘E");
        assert_eq!(titles(all), ["Open", "Edit Quicklink", "Delete Quicklink"]);
        assert_eq!(titles(action_items(&mut ranker, "del", &actions())), ["Delete Quicklink"]);
        assert!(action_items(&mut ranker, "zzz", &actions()).is_empty());
    }

    #[test]
    fn confirm_rows_are_items_in_order() {
        let mut c = Confirm::new("apps", "uninstall/x", "Uninstall X?");
        c.rows = vec![
            ConfirmRow::new("X.app"),
            ConfirmRow {
                subtitle: "~/Library".into(),
                accessory: "2 MB".into(),
                ..ConfirmRow::new("Caches")
            },
        ];
        let items = confirm_items(&c);
        assert_eq!(items.len(), 2);
        assert_eq!((items[1].title.as_str(), items[1].subtitle.as_str()), ("Caches", "~/Library"));
        assert_eq!(items[1].accessory, "2 MB");
    }

    #[test]
    fn hints_name_the_keys() {
        let item = Item::new(ItemId::new("app", "x"), "X", "Open Application", Icon::Symbol("app"));
        assert_eq!(list_hint(Some(&item), false), "Open Application  ↵");
        assert_eq!(list_hint(Some(&item), true), "Open Application  ↵    Actions  ⌘K");
        assert_eq!(list_hint(None, true), "");
        let mut c = Confirm::new("quicklink", "t", "Delete Docs?");
        c.label = "Delete Quicklink".into();
        assert_eq!(confirm_hint(&c), "Delete Quicklink  ↵");
        c.destructive = true;
        assert_eq!(confirm_hint(&c), "Delete Quicklink  ⌘↵");
    }

    #[test]
    fn a_form_names_its_first_blank_required_field() {
        let mut form = Form {
            fields: vec![
                Field::new("name", "Name").required(),
                Field::new("url", "URL").required(),
            ],
            ..Form::new("quicklink", "new", "Create Quicklink")
        };
        assert_eq!(missing(&form).as_deref(), Some("Name is required"));
        form.set_value(0, "Docs");
        assert_eq!(missing(&form).as_deref(), Some("URL is required"));
        form.set_value(1, "https://docs.rs");
        assert_eq!(missing(&form), None);
    }

    #[test]
    fn only_cmd_enter_confirms_a_destructive_question() {
        assert_eq!(confirm_step(false, Key::Enter), ConfirmStep::Confirm);
        assert_eq!(confirm_step(false, Key::CmdEnter), ConfirmStep::Confirm);
        assert_eq!(confirm_step(true, Key::Enter), ConfirmStep::Hint);
        assert_eq!(confirm_step(true, Key::CmdEnter), ConfirmStep::Confirm);
        assert_eq!(confirm_step(true, Key::Escape), ConfirmStep::Cancel);
        assert_eq!(confirm_step(true, Key::Up), ConfirmStep::Scroll(-1));
        assert_eq!(confirm_step(true, Key::Down), ConfirmStep::Scroll(1));
        assert_eq!(confirm_step(false, Key::PageUp), ConfirmStep::Scroll(-4));
        assert_eq!(confirm_step(false, Key::PageDown), ConfirmStep::Scroll(4));
        assert_eq!(confirm_step(false, Key::Top), ConfirmStep::Scroll(isize::MIN));
        assert_eq!(confirm_step(false, Key::Bottom), ConfirmStep::Scroll(isize::MAX));
        for key in [Key::Tab, Key::BackTab, Key::Backspace, Key::CmdK] {
            assert_eq!(confirm_step(false, key), ConfirmStep::Ignore);
        }
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        assert_eq!(scrolled(0, -1, 20, 8), 0);
        assert_eq!(scrolled(0, 1, 20, 8), 1);
        assert_eq!(scrolled(12, 1, 20, 8), 12);
        assert_eq!(scrolled(0, 1, 3, 8), 0);
        // G and gg: the bottom and the top, whatever the length.
        assert_eq!(scrolled(5, isize::MAX, 20, 8), 12);
        assert_eq!(scrolled(5, isize::MIN, 20, 8), 0);
    }

    fn over(screen: Screen) -> Back {
        Back { screen: Box::new(screen), query: String::new(), selected: 0, scroll: 0 }
    }

    #[test]
    fn a_view_closes_when_the_screen_stops_holding_it() {
        let list = |name: &str| Screen::List(ListView::new("sys", name));
        let mut held = None;
        assert_eq!(hold(&mut held, &Screen::Root), None);
        assert_eq!(hold(&mut held, &list("fleet")), None);
        // Opened again (a hotkey on a hidden launcher): still the same view.
        assert_eq!(hold(&mut held, &list("fleet")), None);
        // An action menu and a confirmation over it still hold it.
        let target = Item::new(ItemId::new("sys", "x"), "X", "", Icon::Symbol("app"));
        let actions = Screen::Actions { target, actions: vec![], back: over(list("fleet")) };
        assert_eq!(actions.view().map(|v| v.name.as_str()), Some("fleet"));
        assert_eq!(hold(&mut held, &actions), None);
        let confirm = Confirm::new("sys", "restart/x", "Restart?");
        assert_eq!(hold(&mut held, &Screen::Confirm { confirm, back: over(list("fleet")) }), None);
        // Another view of the same module closes it.
        assert_eq!(hold(&mut held, &list("machine")), Some(("sys", "fleet".into())));
        // So do root search and a form.
        assert_eq!(hold(&mut held, &Screen::Root), Some(("sys", "machine".into())));
        assert_eq!(hold(&mut held, &list("fleet")), None);
        let form = Screen::Form(Form::new("quicklink", "new", "New"));
        assert_eq!(hold(&mut held, &form), Some(("sys", "fleet".into())));
        assert_eq!((held, form.view().is_none()), (None, true));
    }

    #[test]
    fn root_and_lists_have_actions_and_toggle_by_name() {
        let view = ListView::new("clip", "history");
        assert!(Screen::Root.shows(None) && Screen::Root.is_list());
        assert!(!Screen::Root.shows(Some(&view)));
        let list = Screen::List(view.clone());
        assert!(list.shows(Some(&view)) && list.is_list() && !list.shows(None));
        assert!(!list.shows(Some(&ListView::new("clip", "other"))));
        let form = Screen::Form(Form::new("quicklink", "new", "New"));
        assert!(!form.is_list() && !form.shows(None));
    }
}
