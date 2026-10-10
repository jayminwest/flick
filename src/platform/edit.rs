//! The standard edit keys for Flick's key-capable windows. Flick is an accessory app with no
//! main menu, so nothing sends `cut:`, `copy:`, `paste:`, `selectAll:`, `undo:` or `redo:`
//! for ⌘X/⌘C/⌘V/⌘A/⌘Z/⇧⌘Z; each window's `performKeyEquivalent:` calls `send` first
//! (mx-43d850). Used by the launcher `panel`, the `hud` cards and every `surface`.

use objc2::runtime::{AnyObject, Sel};
use objc2::sel;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags};

use super::mtm;

/// The modifiers that count; Caps Lock, Fn and the keypad flag don't.
pub(in crate::platform) fn held(flags: NSEventModifierFlags) -> NSEventModifierFlags {
    flags
        & (NSEventModifierFlags::Shift
            | NSEventModifierFlags::Control
            | NSEventModifierFlags::Option
            | NSEventModifierFlags::Command)
}

/// Whether ⌘ is the only modifier held.
pub(in crate::platform) fn command_only(flags: NSEventModifierFlags) -> bool {
    held(flags) == NSEventModifierFlags::Command
}

/// Whether ⌘ and ⇧ are the only modifiers held.
pub(in crate::platform) fn command_shift(flags: NSEventModifierFlags) -> bool {
    held(flags) == NSEventModifierFlags::Command | NSEventModifierFlags::Shift
}

/// The standard edit action for a key equivalent (`chars` ignores modifiers).
pub(in crate::platform) fn edit_action(
    command_only: bool,
    command_shift: bool,
    chars: &str,
) -> Option<Sel> {
    let c = chars.to_ascii_lowercase();
    Some(match (command_only, command_shift, c.as_str()) {
        (true, _, "x") => sel!(cut:),
        (true, _, "c") => sel!(copy:),
        (true, _, "v") => sel!(paste:),
        (true, _, "a") => sel!(selectAll:),
        (true, _, "z") => sel!(undo:),
        (_, true, "z") => sel!(redo:),
        _ => return None,
    })
}

/// If `event` is an edit key, send its action down the responder chain with `sender` as the
/// sender. True when it was one and a responder took it.
pub(in crate::platform) fn send(event: &NSEvent, sender: &AnyObject) -> bool {
    let flags = event.modifierFlags();
    let chars = event.charactersIgnoringModifiers().map(|s| s.to_string());
    let chars = chars.as_deref().unwrap_or("");
    edit_action(command_only(flags), command_shift(flags), chars).is_some_and(|action| {
        let app = NSApplication::sharedApplication(mtm());
        // SAFETY: a standard edit action, a nil target (the responder chain) and a live sender.
        unsafe { app.sendAction_to_from(action, None, Some(sender)) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_sets_ignore_caps_lock_fn_and_keypad() {
        use NSEventModifierFlags as F;
        assert!(command_only(F::Command));
        assert!(command_only(F::Command | F::CapsLock | F::NumericPad | F::Function));
        assert!(!command_only(F::Command | F::Shift));
        assert!(!command_only(F::Option));
        assert!(command_shift(F::Command | F::Shift | F::CapsLock));
        assert!(!command_shift(F::Command));
        assert!(!command_shift(F::Command | F::Shift | F::Option));
    }

    #[test]
    fn edit_keys_send_the_standard_edit_actions() {
        let pairs = [("x", sel!(cut:)), ("c", sel!(copy:)), ("v", sel!(paste:))];
        for (c, action) in pairs {
            assert_eq!(edit_action(true, false, c), Some(action));
        }
        assert_eq!(edit_action(true, false, "a"), Some(sel!(selectAll:)));
        assert_eq!(edit_action(true, false, "z"), Some(sel!(undo:)));
        assert_eq!(edit_action(false, true, "Z"), Some(sel!(redo:)));
        // Without ⌘ alone (or ⇧⌘ for redo) the key types, and other letters are not edits.
        assert_eq!(edit_action(false, false, "v"), None);
        assert_eq!(edit_action(false, true, "v"), None);
        assert_eq!(edit_action(true, false, "k"), None);
    }
}
