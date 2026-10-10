//! Vim-style navigation keys of the launcher panel (flick-3845). Pure: no `AppKit`.
//!
//! - ⌃J / ⌃N move down and ⌃K / ⌃P move up, also while the search field has the keyboard.
//!   A key the screen does not take (a form) keeps its text-editing default.
//! - With a read-only title (no text input): J / K move, G / ⇧G jump to the top / bottom,
//!   ⌃D / ⌃U move half a page.

use super::Key;

/// The modifiers held with a key, as they decide a navigation key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// No modifier, or ⇧ alone (⇧G).
    Plain,
    /// ⌃ alone.
    Control,
    /// Any other combination.
    Other,
}

/// The navigation key for `chars` (the characters typed, ignoring modifiers except ⇧) with
/// `held`. `read_only`: the panel shows a read-only title, so letters type nothing.
pub fn nav(chars: &str, held: Held, read_only: bool) -> Option<Key> {
    match (held, chars) {
        (Held::Control, "j" | "n") => Some(Key::Down),
        (Held::Control, "k" | "p") => Some(Key::Up),
        (Held::Control, "d") if read_only => Some(Key::PageDown),
        (Held::Control, "u") if read_only => Some(Key::PageUp),
        (Held::Plain, "j") if read_only => Some(Key::Down),
        (Held::Plain, "k") if read_only => Some(Key::Up),
        (Held::Plain, "g") if read_only => Some(Key::Top),
        (Held::Plain, "G") if read_only => Some(Key::Bottom),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_keys_move_the_selection_everywhere() {
        for read_only in [false, true] {
            assert_eq!(nav("j", Held::Control, read_only), Some(Key::Down));
            assert_eq!(nav("n", Held::Control, read_only), Some(Key::Down));
            assert_eq!(nav("k", Held::Control, read_only), Some(Key::Up));
            assert_eq!(nav("p", Held::Control, read_only), Some(Key::Up));
        }
        assert_eq!(nav("j", Held::Other, true), None);
        assert_eq!(nav("a", Held::Control, true), None);
    }

    #[test]
    fn plain_letters_and_half_pages_only_without_a_text_input() {
        assert_eq!(nav("j", Held::Plain, true), Some(Key::Down));
        assert_eq!(nav("k", Held::Plain, true), Some(Key::Up));
        assert_eq!(nav("g", Held::Plain, true), Some(Key::Top));
        assert_eq!(nav("G", Held::Plain, true), Some(Key::Bottom));
        assert_eq!(nav("d", Held::Control, true), Some(Key::PageDown));
        assert_eq!(nav("u", Held::Control, true), Some(Key::PageUp));
        // Typing in the search field: j, k, g, ⌃D (delete forward) and ⌃U keep their default.
        for c in ["j", "k", "g", "G"] {
            assert_eq!(nav(c, Held::Plain, false), None, "{c}");
        }
        assert_eq!(nav("d", Held::Control, false), None);
        assert_eq!(nav("u", Held::Control, false), None);
        assert_eq!(nav("x", Held::Plain, true), None);
    }
}
