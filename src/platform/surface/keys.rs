//! Pure key handling of a surface: `Keystroke`s from key equivalents and text-view commands,
//! and what the surface does with one its handler did not take. No `AppKit`.

/// A key a surface reports to its module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Return,
    Escape,
    Up,
    Down,
    /// A character typed with ⌘ held; letters are lowercase (⇧ is in `shift`).
    Char(char),
}

/// A key and the modifiers held with it. Each module picks its own bindings from these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keystroke {
    pub key: Key,
    pub cmd: bool,
    pub shift: bool,
    pub opt: bool,
}

/// The modifiers held, as booleans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub cmd: bool,
    pub shift: bool,
    pub opt: bool,
}

impl Mods {
    fn with(self, key: Key) -> Keystroke {
        Keystroke { key, cmd: self.cmd, shift: self.shift, opt: self.opt }
    }
}

/// The named key for virtual key `code` (36 Return, 76 keypad Enter, 53 Escape, 126 Up,
/// 125 Down).
fn named(code: u16) -> Option<Key> {
    match code {
        36 | 76 => Some(Key::Return),
        53 => Some(Key::Escape),
        126 => Some(Key::Up),
        125 => Some(Key::Down),
        _ => None,
    }
}

/// The keystroke for a key equivalent (a key pressed with ⌘): `chars` ignores modifiers
/// except ⇧. Without ⌘, or with no single character, there is none.
pub fn equivalent(chars: &str, code: u16, mods: Mods) -> Option<Keystroke> {
    if !mods.cmd {
        return None;
    }
    let key = named(code).or_else(|| {
        let mut it = chars.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Some(Key::Char(c.to_ascii_lowercase())),
            _ => None,
        }
    })?;
    Some(mods.with(key))
}

/// The keystroke for a text-view command selector (its name), if the surface reports it.
pub fn command(selector: &str, mods: Mods) -> Option<Keystroke> {
    let key = match selector {
        "insertNewline:" | "insertNewlineIgnoringFieldEditor:" | "insertLineBreak:" => Key::Return,
        // A text view sends complete: for Esc when nothing handles cancelOperation:.
        "cancelOperation:" | "complete:" => Key::Escape,
        "moveUp:" => Key::Up,
        "moveDown:" => Key::Down,
        _ => return None,
    };
    Some(mods.with(key))
}

/// What the surface does with a keystroke its module's `key` handler did not take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// Send the input's text to `submit` and clear it.
    Submit,
    /// Hide the surface and tell `closed`.
    Close,
    /// Leave it to the text view (a newline, a caret move, a menu-less key equivalent).
    Default,
}

/// Return submits, except ⇧ or ⌥ Return in a multi-line input, which inserts a newline.
/// Escape closes. Everything else keeps its default.
pub fn fallback(k: Keystroke, multi: bool) -> Fallback {
    match k.key {
        Key::Return if multi && !k.cmd && (k.shift || k.opt) => Fallback::Default,
        Key::Return => Fallback::Submit,
        Key::Escape => Fallback::Close,
        _ => Fallback::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: Mods = Mods { cmd: true, shift: false, opt: false };
    const CMD_SHIFT: Mods = Mods { cmd: true, shift: true, opt: false };
    const NONE: Mods = Mods { cmd: false, shift: false, opt: false };

    fn ks(key: Key, mods: Mods) -> Keystroke {
        mods.with(key)
    }

    #[test]
    fn key_equivalents_need_command_and_lowercase_letters() {
        assert_eq!(equivalent("n", 45, CMD), Some(ks(Key::Char('n'), CMD)));
        assert_eq!(equivalent("V", 9, CMD_SHIFT), Some(ks(Key::Char('v'), CMD_SHIFT)));
        assert_eq!(equivalent("[", 33, CMD), Some(ks(Key::Char('['), CMD)));
        assert_eq!(equivalent("\r", 36, CMD), Some(ks(Key::Return, CMD)));
        assert_eq!(equivalent("\u{3}", 76, CMD), Some(ks(Key::Return, CMD)));
        assert_eq!(equivalent("", 53, CMD), Some(ks(Key::Escape, CMD)));
        assert_eq!(equivalent("", 126, CMD), Some(ks(Key::Up, CMD)));
        assert_eq!(equivalent("", 125, CMD), Some(ks(Key::Down, CMD)));
        assert_eq!(equivalent("n", 45, NONE), None);
        assert_eq!(equivalent("", 0, CMD), None);
        assert_eq!(equivalent("ab", 0, CMD), None);
    }

    #[test]
    fn text_view_commands_map_to_keys() {
        let opt = Mods { opt: true, ..NONE };
        for sel in ["insertNewline:", "insertNewlineIgnoringFieldEditor:", "insertLineBreak:"] {
            assert_eq!(command(sel, opt), Some(ks(Key::Return, opt)));
        }
        assert_eq!(command("cancelOperation:", NONE), Some(ks(Key::Escape, NONE)));
        assert_eq!(command("complete:", NONE), Some(ks(Key::Escape, NONE)));
        assert_eq!(command("moveUp:", NONE), Some(ks(Key::Up, NONE)));
        assert_eq!(command("moveDown:", NONE), Some(ks(Key::Down, NONE)));
        assert_eq!(command("deleteBackward:", NONE), None);
    }

    #[test]
    fn return_submits_unless_it_adds_a_line_and_escape_closes() {
        let shift = Mods { shift: true, ..NONE };
        let opt = Mods { opt: true, ..NONE };
        assert_eq!(fallback(ks(Key::Return, NONE), true), Fallback::Submit);
        assert_eq!(fallback(ks(Key::Return, shift), true), Fallback::Default);
        assert_eq!(fallback(ks(Key::Return, opt), true), Fallback::Default);
        assert_eq!(fallback(ks(Key::Return, CMD_SHIFT), true), Fallback::Submit);
        assert_eq!(fallback(ks(Key::Return, shift), false), Fallback::Submit);
        assert_eq!(fallback(ks(Key::Escape, NONE), false), Fallback::Close);
        assert_eq!(fallback(ks(Key::Up, NONE), true), Fallback::Default);
        assert_eq!(fallback(ks(Key::Char('n'), CMD), true), Fallback::Default);
        assert_eq!(Mods::default(), NONE);
    }
}
