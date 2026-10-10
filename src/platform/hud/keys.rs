//! The keys a card answers while it holds the keyboard (`hud::focus_top`, a click in a field):
//! pure mapping from a key-down to a HUD command, the Tab cycle and the move between cards.
//! No `AppKit` here, so all of it is unit-tested (100% floor).
//!
//! - Esc gives the keyboard back to the front app; the card stays (even a card that Esc
//!   would dismiss from another app).
//! - Tab / Shift-Tab move to the next / previous control of the card, wrapping.
//! - ⌘1..⌘6 press action n; ⌘↵ presses the card's primary action (`Card::primary`).
//! - ⌥↑ / ⌥↓ move the keyboard to the card above / below, among the visible ones.
//!
//! Everything else (typing, Space on a focused button, the edit keys) goes on to `AppKit`.

/// Virtual key codes.
const ESCAPE: u16 = 53;
const TAB: u16 = 48;
const RETURN: u16 = 36;
const ENTER: u16 = 76;
const UP: u16 = 126;
const DOWN: u16 = 125;

/// The keys that matter here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Escape,
    Tab,
    /// Return or keypad Enter.
    Return,
    Up,
    Down,
    /// A digit 1 to 9, by the characters the key types without modifiers.
    Digit(u8),
    Other,
}

/// The modifiers held (Caps Lock, Fn and the numeric pad flag do not count).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "one flag per modifier key")]
pub struct Mods {
    pub command: bool,
    pub shift: bool,
    pub option: bool,
    pub control: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { command: false, shift: false, option: false, control: false };
    pub const COMMAND: Mods = Mods { command: true, ..Mods::NONE };
    pub const SHIFT: Mods = Mods { shift: true, ..Mods::NONE };
    pub const OPTION: Mods = Mods { option: true, ..Mods::NONE };
}

/// What a key does to the focused card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Give the keyboard back; the card stays.
    Unfocus,
    /// The next (`false`) or previous (`true`) control.
    Cycle { back: bool },
    /// Press the action at this index (⌘1 is 0).
    Press(usize),
    /// Press the primary action.
    Primary,
    /// Move the keyboard to the card above (`true`) or below.
    Move { up: bool },
}

/// Most actions a card has, and so the highest ⌘ digit that presses one.
pub const MAX_ACTIONS: u8 = 6;

/// The key of virtual key code `code`, which types `chars` without modifiers.
pub fn key(code: u16, chars: &str) -> Key {
    match code {
        ESCAPE => Key::Escape,
        TAB => Key::Tab,
        RETURN | ENTER => Key::Return,
        UP => Key::Up,
        DOWN => Key::Down,
        _ => match chars.as_bytes() {
            [d @ b'1'..=b'9'] => Key::Digit(d - b'0'),
            _ => Key::Other,
        },
    }
}

/// The command of `key` with `mods` held, or `None` to let `AppKit` have the key.
pub fn command(key: Key, mods: Mods) -> Option<Command> {
    match (key, mods) {
        (Key::Escape, Mods::NONE) => Some(Command::Unfocus),
        (Key::Tab, Mods::NONE) => Some(Command::Cycle { back: false }),
        (Key::Tab, Mods::SHIFT) => Some(Command::Cycle { back: true }),
        (Key::Return, Mods::COMMAND) => Some(Command::Primary),
        (Key::Digit(n), Mods::COMMAND) if n <= MAX_ACTIONS => {
            Some(Command::Press(usize::from(n - 1)))
        }
        (Key::Up, Mods::OPTION) => Some(Command::Move { up: true }),
        (Key::Down, Mods::OPTION) => Some(Command::Move { up: false }),
        _ => None,
    }
}

/// The control Tab (or Shift-Tab, `back`) moves to among `len`, from `current` (none: from
/// before the first, or after the last going back). Wraps; `None` when there are none.
pub fn cycle(current: Option<usize>, len: usize, back: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (current.filter(|&c| c < len), back) {
        (None, false) => 0,
        (None, true) => len - 1,
        (Some(c), false) => (c + 1) % len,
        (Some(c), true) => (c + len - 1) % len,
    })
}

/// The card ⌥↑ (`up`) or ⌥↓ moves to from card `current` (newest first) among `visible`
/// cards of a stack in a top corner (`top`: newest at the top, older ones below) or a bottom
/// one (newest at the bottom). Stops at the ends; `None` when there is nowhere to go.
pub fn moved(current: usize, visible: usize, up: bool, top: bool) -> Option<usize> {
    // Toward older cards: down from a top corner, up from a bottom one.
    let older = up != top;
    let next = if older { current.checked_add(1)? } else { current.checked_sub(1)? };
    (next < visible).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_by_code_and_digit() {
        assert_eq!(key(53, "\u{1b}"), Key::Escape);
        assert_eq!(key(48, "\t"), Key::Tab);
        assert_eq!(key(36, "\r"), Key::Return);
        assert_eq!(key(76, "\u{3}"), Key::Return);
        assert_eq!(key(126, ""), Key::Up);
        assert_eq!(key(125, ""), Key::Down);
        assert_eq!(key(18, "1"), Key::Digit(1));
        assert_eq!(key(25, "9"), Key::Digit(9));
        assert_eq!(key(29, "0"), Key::Other);
        assert_eq!(key(0, "a"), Key::Other);
        assert_eq!(key(0, "12"), Key::Other);
        assert_eq!(key(0, ""), Key::Other);
    }

    #[test]
    fn plain_escape_and_tab() {
        assert_eq!(command(Key::Escape, Mods::NONE), Some(Command::Unfocus));
        assert_eq!(command(Key::Escape, Mods::COMMAND), None);
        assert_eq!(command(Key::Tab, Mods::NONE), Some(Command::Cycle { back: false }));
        assert_eq!(command(Key::Tab, Mods::SHIFT), Some(Command::Cycle { back: true }));
        assert_eq!(command(Key::Tab, Mods::OPTION), None);
        let control = Mods { control: true, ..Mods::NONE };
        assert_eq!(command(Key::Tab, control), None);
    }

    #[test]
    fn command_digits_and_return_press() {
        assert_eq!(command(Key::Digit(1), Mods::COMMAND), Some(Command::Press(0)));
        assert_eq!(command(Key::Digit(6), Mods::COMMAND), Some(Command::Press(5)));
        assert_eq!(command(Key::Digit(7), Mods::COMMAND), None);
        assert_eq!(command(Key::Digit(1), Mods::NONE), None, "typing a digit in a field");
        let cmd_shift = Mods { shift: true, ..Mods::COMMAND };
        assert_eq!(command(Key::Digit(1), cmd_shift), None);
        assert_eq!(command(Key::Return, Mods::COMMAND), Some(Command::Primary));
        assert_eq!(command(Key::Return, Mods::NONE), None, "Return stays the field's");
        assert_eq!(command(Key::Other, Mods::COMMAND), None, "⌘C and friends go on");
    }

    #[test]
    fn option_arrows_move() {
        assert_eq!(command(Key::Up, Mods::OPTION), Some(Command::Move { up: true }));
        assert_eq!(command(Key::Down, Mods::OPTION), Some(Command::Move { up: false }));
        assert_eq!(command(Key::Up, Mods::NONE), None, "arrows stay the field's");
        assert_eq!(command(Key::Down, Mods::COMMAND), None);
    }

    #[test]
    fn tab_wraps_both_ways() {
        assert_eq!(cycle(None, 0, false), None);
        assert_eq!(cycle(Some(0), 0, true), None);
        assert_eq!(cycle(None, 3, false), Some(0));
        assert_eq!(cycle(None, 3, true), Some(2));
        assert_eq!(cycle(Some(0), 3, false), Some(1));
        assert_eq!(cycle(Some(2), 3, false), Some(0));
        assert_eq!(cycle(Some(0), 3, true), Some(2));
        assert_eq!(cycle(Some(2), 3, true), Some(1));
        assert_eq!(cycle(Some(9), 3, false), Some(0), "a stale index starts over");
    }

    #[test]
    fn moves_follow_the_corner() {
        // Top corner: newest (0) at the top; ⌥↓ goes to older cards.
        assert_eq!(moved(0, 3, false, true), Some(1));
        assert_eq!(moved(1, 3, true, true), Some(0));
        assert_eq!(moved(0, 3, true, true), None);
        assert_eq!(moved(2, 3, false, true), None, "the last visible card");
        // Bottom corner: newest at the bottom; ⌥↑ goes to older cards.
        assert_eq!(moved(0, 3, true, false), Some(1));
        assert_eq!(moved(1, 3, false, false), Some(0));
        assert_eq!(moved(0, 3, false, false), None);
        assert_eq!(moved(usize::MAX, usize::MAX, true, false), None);
    }
}
