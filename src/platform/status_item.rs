//! Flick's menu bar items (`NSStatusItem`): one per owner (a module id), each a short symbol
//! with a tooltip and a small menu. Flick is an accessory app with no menu bar of its own, so
//! these are its only menu bar presence; an owner's item exists only between `show` and
//! `hide`. Opening a menu does not activate Flick or take key status from the launcher panel.
//!
//! Menu handlers run on the main thread while `AppKit` is mid-event: they only set a flag and
//! post an event, or (for `Entry::Open`) hand off through the opener, which defers its work
//! (mulch mx-fcbc43).

use std::cell::{Cell, OnceCell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

/// One row of an owner's menu.
#[derive(Clone, Copy, Debug)]
pub enum Entry<'a> {
    /// A disabled line of text.
    Info(&'a str),
    /// A divider line.
    Separator,
    /// Calls the owner's pick handler with `key`.
    Pick { title: &'a str, key: &'a str },
    /// Runs module `module`'s hotkey `key` through the opener `set_opener` installed.
    Open { title: &'a str, module: &'a str, key: &'a str },
}

/// What a clicked row does, copied out of the menu before it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Pick(String),
    Open(String, String),
}

/// An owner's item and handlers. Kept after `hide` so the slot (and its tags) stay stable and
/// the menu-open hook survives.
struct Owner {
    id: &'static str,
    item: Option<Retained<NSStatusItem>>,
    /// By menu row; `None` for rows that do nothing.
    actions: Vec<Option<Action>>,
    on_pick: fn(&str),
    on_open: Option<fn()>,
}

/// A pick handler, given the row's key.
type Pick = fn(&str);
/// The opener, given `(module, key)`.
type Opener = fn(&str, &str);

/// A menu item's tag is `slot << ROW_BITS | row`.
const ROW_BITS: u32 = 8;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickStatusItemTarget"]
    struct Target;

    // SAFETY: the protocol's methods are optional; this class implements none of them.
    unsafe impl NSObjectProtocol for Target {}

    // SAFETY: `menuWillOpen:` matches the protocol's signature; the rest are optional.
    unsafe impl NSMenuDelegate for Target {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, menu: &NSMenu) {
            let mtm = self.mtm();
            let hook = OWNERS.with_borrow(|owners| {
                owners.iter().find_map(|o| {
                    let mine = o.item.as_ref().and_then(|i| i.menu(mtm));
                    mine.filter(|m| std::ptr::eq(&**m, menu)).and(o.on_open)
                })
            });
            if let Some(hook) = hook {
                hook();
            }
        }
    }

    impl Target {
        #[unsafe(method(itemClicked:))]
        fn item_clicked(&self, sender: &NSMenuItem) {
            // Copy the action out first: it may call `show` or `hide`, which replace OWNERS.
            let found = OWNERS.with_borrow(|owners| resolve(owners, sender.tag()));
            match found {
                Some((Action::Pick(key), pick)) => pick(&key),
                Some((Action::Open(module, key), _)) => {
                    if let Some(open) = OPENER.get() {
                        open(&module, &key);
                    }
                }
                None => {}
            }
        }
    }
);

thread_local! {
    /// Every owner that has shown an item or set a hook, in first-use order.
    static OWNERS: RefCell<Vec<Owner>> = const { RefCell::new(Vec::new()) };
    /// Routes `Entry::Open` rows; installed once by the controller.
    static OPENER: Cell<Option<Opener>> = const { Cell::new(None) };
    /// The menu items' target and menus' delegate; both are held weakly, so this keeps it.
    static TARGET: OnceCell<Retained<Target>> = const { OnceCell::new() };
}

/// Show `owner`'s item with `symbol` as its title ("●"), `tooltip`, and `menu`. Calling it
/// again while shown updates the same item and menu in place, so an open menu shows the
/// new rows. A `Pick` row calls `on_pick(key)`.
pub fn show(owner: &'static str, symbol: &str, tooltip: &str, menu: &[Entry], on_pick: fn(&str)) {
    let mtm = super::mtm();
    let target = TARGET.with(|t| t.get_or_init(|| new_target(mtm)).clone());
    let (item, ns_menu) = OWNERS.with_borrow_mut(|owners| {
        let slot = slot(owners, owner);
        let o = &mut owners[slot];
        o.on_pick = on_pick;
        o.actions = menu.iter().map(action).collect();
        let item = o.item.get_or_insert_with(|| {
            NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength)
        });
        let ns_menu = item.menu(mtm).unwrap_or_else(|| {
            let m = NSMenu::new(mtm);
            m.setDelegate(Some(ProtocolObject::from_ref(&*target)));
            m
        });
        ns_menu.removeAllItems();
        for (row, entry) in menu.iter().enumerate() {
            ns_menu.addItem(&menu_item(mtm, &target, entry, tag(slot, row)));
        }
        (item.clone(), ns_menu)
    });
    if let Some(button) = item.button(mtm) {
        button.setTitle(&NSString::from_str(symbol));
        button.setToolTip(Some(&NSString::from_str(tooltip)));
    }
    item.setMenu(Some(&ns_menu));
}

/// Remove `owner`'s item from the menu bar. Does nothing when it is not shown.
pub fn hide(owner: &str) {
    let item = OWNERS.with_borrow_mut(|owners| {
        owners.iter_mut().find(|o| o.id == owner).and_then(|o| {
            o.actions.clear();
            o.item.take()
        })
    });
    if let Some(item) = item {
        NSStatusBar::systemStatusBar().removeStatusItem(&item);
    }
}

/// Call `hook` each time `owner`'s menu is about to open (before or after `show`). It runs
/// while `AppKit` tracks the menu: set a flag and post an event, never borrow app state.
pub fn on_menu_open(owner: &'static str, hook: fn()) {
    OWNERS.with_borrow_mut(|owners| {
        let slot = slot(owners, owner);
        owners[slot].on_open = Some(hook);
    });
}

/// Route every `Entry::Open` row to `open(module, key)`. The controller installs it once; it
/// runs mid-event, so it must defer its work (`events::on_main`).
pub fn set_opener(open: Opener) {
    OPENER.set(Some(open));
}

/// Whether `owner`'s item is in the menu bar.
#[cfg(test)]
pub fn shown(owner: &str) -> bool {
    OWNERS.with_borrow(|owners| owners.iter().any(|o| o.id == owner && o.item.is_some()))
}

fn new_target(mtm: objc2::MainThreadMarker) -> Retained<Target> {
    // SAFETY: plain `init` on a freshly allocated `NSObject` subclass with no ivars.
    unsafe { msg_send![Target::alloc(mtm), init] }
}

fn menu_item(
    mtm: objc2::MainThreadMarker,
    target: &Target,
    entry: &Entry,
    tag: isize,
) -> Retained<NSMenuItem> {
    let title = match *entry {
        Entry::Separator => return NSMenuItem::separatorItem(mtm),
        Entry::Info(title) | Entry::Pick { title, .. } | Entry::Open { title, .. } => title,
    };
    let act = (!matches!(entry, Entry::Info(_))).then_some(sel!(itemClicked:));
    // SAFETY: `itemClicked:` is a method of `Target`, the item's target, and takes the
    // sending menu item. A row with no action is shown disabled.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            act,
            &NSString::from_str(""),
        )
    };
    if act.is_some() {
        // SAFETY: `target` is kept alive by TARGET for the life of the process.
        unsafe { item.setTarget(Some(target)) };
        item.setTag(tag);
    }
    item
}

/// `owner`'s index in `owners`, adding it (with no item and no-op handlers) if new.
fn slot(owners: &mut Vec<Owner>, owner: &'static str) -> usize {
    owners.iter().position(|o| o.id == owner).unwrap_or_else(|| {
        owners.push(Owner {
            id: owner,
            item: None,
            actions: vec![],
            on_pick: |_| {},
            on_open: None,
        });
        owners.len() - 1
    })
}

fn action(entry: &Entry) -> Option<Action> {
    match *entry {
        Entry::Pick { key, .. } => Some(Action::Pick(key.to_owned())),
        Entry::Open { module, key, .. } => Some(Action::Open(module.to_owned(), key.to_owned())),
        Entry::Info(_) | Entry::Separator => None,
    }
}

fn tag(slot: usize, row: usize) -> isize {
    ((slot << ROW_BITS) | row) as isize
}

/// The action and pick handler behind the menu item with `tag`; `None` for a tag no shown
/// menu has.
fn resolve(owners: &[Owner], tag: isize) -> Option<(Action, Pick)> {
    let tag = usize::try_from(tag).ok()?;
    let owner = owners.get(tag >> ROW_BITS)?;
    let action = owner.actions.get(tag & ((1 << ROW_BITS) - 1))?.clone()?;
    Some((action, owner.on_pick))
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static PICKED: RefCell<String> = const { RefCell::new(String::new()) };
    }
    fn pick_a(key: &str) {
        PICKED.set(format!("a:{key}"));
    }
    fn pick_b(key: &str) {
        PICKED.set(format!("b:{key}"));
    }
    fn noop() {}

    fn owners() -> Vec<Owner> {
        let mut owners = vec![];
        for (id, pick, menu) in [
            (
                "a",
                pick_a as fn(&str),
                &[Entry::Info("x"), Entry::Pick { title: "S", key: "stop" }][..],
            ),
            (
                "b",
                pick_b,
                &[
                    Entry::Separator,
                    Entry::Open { title: "Ask", module: "kota", key: "ask" },
                    Entry::Pick { title: "R", key: "refresh" },
                ],
            ),
        ] {
            let s = slot(&mut owners, id);
            owners[s].on_pick = pick;
            owners[s].actions = menu.iter().map(action).collect();
        }
        owners
    }

    #[test]
    fn tags_find_the_owner_and_row() {
        let owners = owners();
        let (act, pick) = resolve(&owners, tag(0, 1)).unwrap();
        assert_eq!(act, Action::Pick("stop".into()));
        pick("stop");
        assert_eq!(PICKED.with_borrow(Clone::clone), "a:stop");
        let (act, pick) = resolve(&owners, tag(1, 2)).unwrap();
        assert_eq!(act, Action::Pick("refresh".into()));
        pick("refresh");
        assert_eq!(PICKED.with_borrow(Clone::clone), "b:refresh");
        let open = resolve(&owners, tag(1, 1)).unwrap().0;
        assert_eq!(open, Action::Open("kota".into(), "ask".into()));
    }

    #[test]
    fn rows_without_actions_and_unknown_tags_resolve_to_nothing() {
        let owners = owners();
        for t in [tag(0, 0), tag(1, 0), tag(0, 2), tag(2, 0), -1] {
            assert!(resolve(&owners, t).is_none(), "tag {t}");
        }
        assert!(resolve(&[], 0).is_none());
    }

    #[test]
    fn slots_are_stable_per_owner() {
        let mut owners = owners();
        assert_eq!(slot(&mut owners, "b"), 1);
        assert_eq!(slot(&mut owners, "c"), 2);
        assert_eq!(slot(&mut owners, "a"), 0);
        (owners[2].on_pick)("ignored");
        assert!(owners[2].on_open.is_none() && owners[2].actions.is_empty());
    }

    #[test]
    fn hooks_register_without_showing_anything() {
        on_menu_open("hooked", noop);
        set_opener(|_, _| {});
        assert!(!shown("hooked") && !shown("activity"));
        hide("hooked");
        hide("never");
        assert!(!shown("hooked"));
        let hooked =
            OWNERS.with_borrow(|o| o.iter().any(|o| o.id == "hooked" && o.on_open.is_some()));
        assert!(hooked);
    }
}
