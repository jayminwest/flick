//! `[message]` settings: the table `Module::configure` reads, with its defaults.

use serde::Deserialize;

use crate::platform::hud::Corner;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Style {
    /// The corner panel.
    #[default]
    Panel,
    /// A system notification.
    Notification,
    Both,
    /// History only.
    None,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Position {
    #[default]
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
    Top,
    Bottom,
}

impl Position {
    pub(super) fn corner(self) -> Corner {
        match self {
            Position::TopRight => Corner::TopRight,
            Position::TopLeft => Corner::TopLeft,
            Position::BottomRight => Corner::BottomRight,
            Position::BottomLeft => Corner::BottomLeft,
            Position::Top => Corner::Top,
            Position::Bottom => Corner::Bottom,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Settings {
    /// The root item's title and the panel header of a post with no `--title`.
    pub(super) name: String,
    pub(super) style: Style,
    pub(super) position: Position,
    /// Panel width in points, 240 to 900.
    pub(super) width: f64,
    /// Seconds the panel stays; 0 keeps it until dismissed.
    pub(super) timeout_secs: u64,
    /// Cards shown at once; older ones collapse into a `+N more` pill.
    pub(super) max_cards: usize,
    /// Messages kept in history outside threads (`post` without `--thread`).
    pub(super) max_history: usize,
    /// Messages kept per thread (`post --thread`, a card's `thread`).
    pub(super) chat_history: usize,
    /// Threads kept; the one with the oldest last message goes first, all of it.
    pub(super) chat_threads: usize,
    /// Play a short sound when a reply arrives (never for pending posts).
    pub(super) sound: bool,
    /// Opens the message list.
    pub(super) hotkey: Option<String>,
    /// Moves the keyboard into the newest card (or gives it back).
    pub(super) card_hotkey: Option<String>,
    /// What a reply press runs (argv), with `--action --card <id> --action-id <aid>` appended
    /// and the values JSON on stdin. Empty: a press shows an error naming this key.
    pub(super) action_command: Vec<String>,
    /// Seconds a sent press waits for KOTA's update before the card shows an error; 0 waits.
    pub(super) pending_timeout_secs: u64,
    /// Seconds an open card with actions stays; 0 keeps it until acted on or closed.
    pub(super) card_timeout_secs: u64,
    /// Shows or hides the KOTA chat window (plan pl-75d3); unbound: chat stays inert.
    pub(super) chat_hotkey: Option<String>,
    /// The ssh target a chat ask runs kota-ask on (as `[kota] ssh`).
    pub(super) kota_host: String,
    /// kota-ask on that host, relative to its home (as `[kota] kota_ask`).
    pub(super) kota_ask: String,
    /// Where chat screenshots go on that host, relative to its home (`chat::attach::dir`).
    pub(super) attach_dir: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: "Messages".into(),
            style: Style::Panel,
            position: Position::TopRight,
            width: 380.0,
            timeout_secs: 20,
            max_cards: 4,
            max_history: 50,
            chat_history: 200,
            chat_threads: 20,
            sound: true,
            hotkey: None,
            card_hotkey: None,
            action_command: vec![],
            pending_timeout_secs: 120,
            card_timeout_secs: 0,
            chat_hotkey: None,
            kota_host: super::chat::ask::DEFAULT_HOST.into(),
            kota_ask: super::chat::ask::DEFAULT_KOTA_ASK.into(),
            attach_dir: super::chat::attach::DEFAULT_DIR.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_position_is_its_corner() {
        let all = [
            (Position::TopRight, Corner::TopRight),
            (Position::TopLeft, Corner::TopLeft),
            (Position::BottomRight, Corner::BottomRight),
            (Position::BottomLeft, Corner::BottomLeft),
            (Position::Top, Corner::Top),
            (Position::Bottom, Corner::Bottom),
        ];
        for (p, c) in all {
            assert_eq!(p.corner(), c);
        }
    }
}
