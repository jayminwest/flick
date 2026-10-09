//! Flick's menu bar item (`NSStatusItem`): a short symbol with a tooltip and a small menu.
//! Flick is an accessory app with no menu bar of its own, so this is its only menu bar
//! presence, and it exists only between `show` and `hide`. Opening the menu does not activate
//! Flick or take key status from the launcher panel.

use std::cell::{OnceCell, RefCell};

use objc2::rc::Retained;
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FlickStatusItemTarget"]
    struct Target;

    // SAFETY: the protocol's methods are optional; this class implements none of them.
    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(itemClicked:))]
        fn item_clicked(&self, sender: &NSMenuItem) {
            // Copy the handler out first: it may call `show` or `hide`, which replace ACTIONS.
            let action = ACTIONS.with_borrow(|actions| action_at(actions, sender.tag()));
            if let Some(action) = action {
                action();
            }
        }
    }
);

thread_local! {
    /// The visible item; `None` while hidden.
    static ITEM: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
    /// The menu's handlers, indexed by each menu item's tag.
    static ACTIONS: RefCell<Vec<fn()>> = const { RefCell::new(Vec::new()) };
    /// The menu items' target; a menu item holds its target weakly, so this keeps it alive.
    static TARGET: OnceCell<Retained<Target>> = const { OnceCell::new() };
}

/// Show the item with `symbol` as its title ("●"), `tooltip`, and a menu with one entry per
/// `(title, handler)`. Calling it again while shown updates the same item in place. Handlers
/// run on the main thread while `AppKit` is mid-event: post an event rather than borrow app
/// state (mulch mx-fcbc43).
pub fn show(symbol: &str, tooltip: &str, menu: &[(&str, fn())]) {
    let mtm = super::mtm();
    let item = ITEM.with_borrow_mut(|item| {
        item.get_or_insert_with(|| {
            NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength)
        })
        .clone()
    });
    if let Some(button) = item.button(mtm) {
        button.setTitle(&NSString::from_str(symbol));
        button.setToolTip(Some(&NSString::from_str(tooltip)));
    }
    let target = TARGET.with(|t| t.get_or_init(|| new_target(mtm)).clone());
    let ns_menu = NSMenu::new(mtm);
    for (tag, (title, _)) in menu.iter().enumerate() {
        // SAFETY: `itemClicked:` is a method of `Target`, the item's target, and takes the
        // sending menu item.
        let entry = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(sel!(itemClicked:)),
                &NSString::from_str(""),
            )
        };
        // SAFETY: `target` is kept alive by TARGET for the life of the process.
        unsafe { entry.setTarget(Some(&target)) };
        entry.setTag(tag as isize);
        ns_menu.addItem(&entry);
    }
    ACTIONS.set(menu.iter().map(|&(_, action)| action).collect());
    item.setMenu(Some(&ns_menu));
}

/// Remove the item from the menu bar. Does nothing when it is not shown.
pub fn hide() {
    if let Some(item) = ITEM.take() {
        NSStatusBar::systemStatusBar().removeStatusItem(&item);
    }
    ACTIONS.set(Vec::new());
}

/// Whether the item is in the menu bar.
#[cfg(test)]
pub fn shown() -> bool {
    ITEM.with_borrow(Option::is_some)
}

fn new_target(mtm: objc2::MainThreadMarker) -> Retained<Target> {
    // SAFETY: plain `init` on a freshly allocated `NSObject` subclass with no ivars.
    unsafe { msg_send![Target::alloc(mtm), init] }
}

/// The handler for the menu item with `tag`; `None` for a tag the menu does not have.
fn action_at(actions: &[fn()], tag: isize) -> Option<fn()> {
    usize::try_from(tag).ok().and_then(|i| actions.get(i).copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static CALLED: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    }
    fn first() {
        CALLED.set(1);
    }
    fn second() {
        CALLED.set(2);
    }

    #[test]
    fn tags_index_the_handlers() {
        let actions: [fn(); 2] = [first, second];
        for (tag, want) in [(0, 1), (1, 2)] {
            action_at(&actions, tag).unwrap()();
            assert_eq!(CALLED.get(), want);
        }
        assert!(action_at(&actions, 2).is_none());
        assert!(action_at(&actions, -1).is_none());
        assert!(action_at(&[], 0).is_none());
    }

    #[test]
    fn nothing_is_shown_until_show_is_called() {
        assert!(!shown());
        hide();
        assert!(!shown());
    }
}
