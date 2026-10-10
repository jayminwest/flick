//! The pure rules of a private surface (`Spec::private`): which text services its input turns
//! off, and which edit keys the surface handles itself so that text never reaches the general
//! pasteboard unmarked. No `AppKit`; `input` and `window` apply them.

/// A text service of the input that can learn from, keep or send what is typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    /// Continuous spell checking (and its learned words).
    SpellCheck,
    Grammar,
    /// Automatic spelling correction.
    Autocorrect,
    /// Text completion.
    Completion,
    /// Inline predictions (macOS 14 and later).
    Prediction,
    /// Writing Tools (macOS 15 and later).
    WritingTools,
    /// Math results (macOS 15 and later).
    Math,
    LinkDetection,
    DataDetection,
    /// The undo stack, which keeps the text of every edit while the window lives.
    Undo,
}

/// Every service a private input turns off.
pub const ALL: [Service; 10] = [
    Service::SpellCheck,
    Service::Grammar,
    Service::Autocorrect,
    Service::Completion,
    Service::Prediction,
    Service::WritingTools,
    Service::Math,
    Service::LinkDetection,
    Service::DataDetection,
    Service::Undo,
];

/// The services an input turns off: all of them when private; none (the system defaults)
/// otherwise.
pub fn off(private: bool) -> &'static [Service] {
    if private { &ALL } else { &[] }
}

/// What a private surface does with an edit key's action (`edit::edit_action`'s selector
/// name) instead of sending it down the responder chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Not an edit the surface takes over: send it as usual.
    Pass,
    /// Copy the selection with `pasteboard::set_text_concealed`; `cut` also deletes it.
    Conceal { cut: bool },
    /// Nothing selected: offer ⌘C/⌘X to the module (`Handlers::key`), never the pasteboard.
    Module,
}

/// How a private surface handles `action` with `selected` characters selected.
pub fn edit(action: &str, selected: usize) -> Edit {
    let cut = match action {
        "copy:" => false,
        "cut:" => true,
        _ => return Edit::Pass,
    };
    if selected == 0 { Edit::Module } else { Edit::Conceal { cut } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_input_turns_every_service_off_and_a_normal_one_none() {
        assert_eq!(off(true), ALL);
        assert!(off(false).is_empty());
        for (i, s) in ALL.iter().enumerate() {
            assert!(!ALL[..i].contains(s), "{s:?} listed twice");
        }
    }

    #[test]
    fn copy_and_cut_are_concealed_or_go_to_the_module() {
        assert_eq!(edit("copy:", 3), Edit::Conceal { cut: false });
        assert_eq!(edit("cut:", 1), Edit::Conceal { cut: true });
        assert_eq!(edit("copy:", 0), Edit::Module);
        assert_eq!(edit("cut:", 0), Edit::Module);
        for other in ["paste:", "selectAll:", "undo:", "redo:", ""] {
            assert_eq!(edit(other, 5), Edit::Pass, "{other}");
        }
    }
}
