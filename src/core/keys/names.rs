//! Key names as config spells them: modifiers (`cmd`, `right_alt`, `fn`, ...), `caps_lock`,
//! and the global-hotkey key names (`KeyA`, `Digit1`, `Space`, `F18`, ...), mapped to
//! `CGEventFlags` masks and macOS virtual keycodes.

use std::str::FromStr;

use super::{Chord, flags};

/// One parsed key name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyName {
    /// A modifier: the `CGEventFlags` bits that are all set while it is held. A sided name
    /// (`right_cmd`) adds its device bit, so a left key never matches it.
    Mod(u64),
    /// Any other key, as a macOS virtual keycode.
    Key(u16),
}

impl KeyName {
    #[cfg(test)]
    pub fn is_modifier(self) -> bool {
        matches!(self, KeyName::Mod(_))
    }
}

impl FromStr for KeyName {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, String> {
        if let Some(mask) = modifier(&name.to_ascii_lowercase()) {
            return Ok(KeyName::Mod(mask));
        }
        keycode(name).map(KeyName::Key).ok_or_else(|| format!("unknown key name `{name}`"))
    }
}

fn modifier(name: &str) -> Option<u64> {
    use flags::{ALT, CMD, CTRL, FN, SHIFT};
    Some(match name {
        "cmd" | "command" => CMD,
        "alt" | "option" | "opt" => ALT,
        "ctrl" | "control" => CTRL,
        "shift" => SHIFT,
        "fn" => FN,
        "left_cmd" => CMD | flags::LEFT_CMD,
        "right_cmd" => CMD | flags::RIGHT_CMD,
        "left_alt" => ALT | flags::LEFT_ALT,
        "right_alt" => ALT | flags::RIGHT_ALT,
        "left_ctrl" => CTRL | flags::LEFT_CTRL,
        "right_ctrl" => CTRL | flags::RIGHT_CTRL,
        "left_shift" => SHIFT | flags::LEFT_SHIFT,
        "right_shift" => SHIFT | flags::RIGHT_SHIFT,
        _ => return None,
    })
}

/// Virtual keycodes (`kVK_*`) of the letters A-Z and digits 0-9, in order.
const LETTERS: [u16; 26] =
    [0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16, 6];
const DIGITS: [u16; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
const FKEYS: [u16; 20] =
    [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80, 90];

/// The macOS virtual keycode of a non-modifier key name.
pub fn keycode(name: &str) -> Option<u16> {
    let indexed = |rest: &str, table: &[u16], base: u8| -> Option<u16> {
        match rest.as_bytes() {
            [c] => table.get(usize::from(c.checked_sub(base)?)).copied(),
            _ => None,
        }
    };
    if let Some(rest) = name.strip_prefix("Key") {
        return indexed(rest, &LETTERS, b'A');
    }
    if let Some(rest) = name.strip_prefix("Digit") {
        return indexed(rest, &DIGITS, b'0');
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<usize>().ok()) {
        return FKEYS.get(n.checked_sub(1)?).copied();
    }
    Some(match name {
        "caps_lock" | "CapsLock" => super::CAPS_LOCK,
        "Space" => 49,
        "Escape" => 53,
        "Enter" => 36,
        "Tab" => 48,
        "Backspace" => 51,
        "Delete" => 117,
        "Home" => 115,
        "End" => 119,
        "PageUp" => 116,
        "PageDown" => 121,
        "ArrowLeft" => 123,
        "ArrowRight" => 124,
        "ArrowDown" => 125,
        "ArrowUp" => 126,
        "Minus" => 27,
        "Equal" => 24,
        "BracketLeft" => 33,
        "BracketRight" => 30,
        "Backslash" => 42,
        "Semicolon" => 41,
        "Quote" => 39,
        "Comma" => 43,
        "Period" => 47,
        "Slash" => 44,
        "Backquote" => 50,
        _ => return None,
    })
}

/// Parse `cmd+ctrl+alt+shift` into the flags to add, e.g. for `hyper_mods`.
pub fn parse_mods(spec: &str) -> Result<u64, String> {
    spec.split('+').try_fold(0, |acc, name| match name.trim().parse()? {
        KeyName::Mod(mask) => Ok(acc | mask),
        KeyName::Key(_) => Err(format!("`{}` is not a modifier", name.trim())),
    })
}

/// Parse a chord's key names: any modifiers plus at most one other key.
pub fn parse_chord<S: AsRef<str>>(names: &[S]) -> Result<Chord, String> {
    if names.is_empty() {
        return Err("a chord needs at least one key".into());
    }
    let mut chord = Chord { mods: 0, key: None };
    for name in names {
        match name.as_ref().parse()? {
            KeyName::Mod(mask) => chord.mods |= mask,
            KeyName::Key(code) if chord.key.is_none() => chord.key = Some(code),
            KeyName::Key(_) => return Err("a chord takes at most one non-modifier key".into()),
        }
    }
    Ok(chord)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::keys::{F18, HYPER_FLAGS};

    #[test]
    fn modifiers_parse_to_flag_masks() {
        assert_eq!("cmd".parse(), Ok(KeyName::Mod(flags::CMD)));
        assert_eq!("Option".parse(), Ok(KeyName::Mod(flags::ALT)));
        assert_eq!("right_cmd".parse(), Ok(KeyName::Mod(flags::CMD | flags::RIGHT_CMD)));
        assert_eq!("left_alt".parse(), Ok(KeyName::Mod(flags::ALT | flags::LEFT_ALT)));
        for name in ["command", "alt", "opt", "ctrl", "control", "shift", "fn", "left_cmd"] {
            assert!(name.parse::<KeyName>().unwrap().is_modifier(), "{name}");
        }
        for name in ["right_alt", "left_ctrl", "right_ctrl", "left_shift", "right_shift"] {
            assert!(name.parse::<KeyName>().unwrap().is_modifier(), "{name}");
        }
    }

    #[test]
    fn keys_parse_to_virtual_keycodes() {
        let cases = [
            ("KeyA", 0),
            ("KeyH", 4),
            ("KeyZ", 6),
            ("Digit0", 29),
            ("Digit9", 25),
            ("F1", 122),
            ("F18", F18),
            ("F20", 90),
            ("caps_lock", 57),
            ("CapsLock", 57),
            ("Space", 49),
            ("Escape", 53),
        ];
        for (name, code) in cases {
            assert_eq!(name.parse(), Ok(KeyName::Key(code)), "{name}");
            assert!(!KeyName::Key(code).is_modifier());
        }
        let named = [
            "Enter",
            "Tab",
            "Backspace",
            "Delete",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "ArrowLeft",
            "ArrowRight",
            "ArrowDown",
            "ArrowUp",
            "Minus",
            "Equal",
            "BracketLeft",
            "BracketRight",
            "Backslash",
            "Semicolon",
            "Quote",
            "Comma",
            "Period",
            "Slash",
            "Backquote",
        ];
        for name in named {
            assert!(keycode(name).is_some(), "{name}");
        }
    }

    #[test]
    fn unknown_names_are_errors() {
        for name in ["", "Key", "KeyAA", "Keya", "Digit", "DigitX", "F0", "F21", "Fx", "hyper"] {
            assert!(name.parse::<KeyName>().is_err(), "{name}");
        }
        assert_eq!("nope".parse::<KeyName>(), Err("unknown key name `nope`".into()));
    }

    #[test]
    fn mods_specs_combine_and_reject_keys() {
        assert_eq!(parse_mods("cmd+ctrl+alt+shift"), Ok(HYPER_FLAGS));
        assert_eq!(parse_mods(" cmd + shift "), Ok(flags::CMD | flags::SHIFT));
        assert_eq!(parse_mods("cmd+KeyA"), Err("`KeyA` is not a modifier".into()));
        assert!(parse_mods("cmd+bogus").is_err());
    }

    #[test]
    fn chords_take_modifiers_and_one_key() {
        let chord = parse_chord(&["right_cmd", "right_alt"]).unwrap();
        assert_eq!(chord.mods, flags::CMD | flags::RIGHT_CMD | flags::ALT | flags::RIGHT_ALT);
        assert_eq!(chord.key, None);
        let chord = parse_chord(&["ctrl", "KeyA"]).unwrap();
        assert_eq!((chord.mods, chord.key), (flags::CTRL, Some(0)));
        assert!(parse_chord::<&str>(&[]).is_err());
        assert!(parse_chord(&["KeyA", "KeyB"]).is_err());
        assert!(parse_chord(&["cmd", "bogus"]).is_err());
    }
}
