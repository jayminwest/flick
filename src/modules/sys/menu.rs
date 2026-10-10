//! The fleet window's cmd+K actions (flick-1e00), pure: one card per machine under its
//! bubble with the machine's cmd+K menu of the launcher as buttons, and what a press on it
//! means. No locks, no I/O; `window.rs` draws the cards and runs the presses.
//!
//! - A card's id is `actions/<machine>`. Its buttons: Screen Sharing (`vnc`), Open Dash
//!   (`dash`), Tail <service> (`tail/<service>`, a service with `log`) and Restart
//!   <service>… (`restart/<service>`, a service with `restart = true`), only for services
//!   that this Mac's config defines (`act::resolve`), as in the launcher.
//! - Restart is a card `shell` action showing the exact command, so the first press only
//!   asks: the card shows the command with Cancel and Run (`CardUi::confirm`). Run presses
//!   the same action again and restarts, re-resolved from config: the shown text never runs.
//!   A restart that cannot run (no uid) is a disabled button with the reason.

use serde_json::Value;

use super::act::Target;
use super::settings::Machine;
use crate::core::card::{Action, Card, Do, Kind, State, Style};

/// The key and card id prefix of a machine's card.
const PREFIX: &str = "actions/";

/// The card id of `machine`'s actions.
pub fn card_id(machine: &str) -> String {
    format!("{PREFIX}{machine}")
}

/// What a press on a card asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Press<'a> {
    /// Screen Sharing (`vnc`) or Open Dash (`dash`) of the machine.
    Open { machine: &'a str, vnc: bool },
    Tail { machine: &'a str, service: &'a str },
    Restart { machine: &'a str, service: &'a str },
    /// Cancel on the restart's confirm.
    Cancel,
}

/// The press of action `action` on card `card`; `cancel` is the id Cancel presses
/// (`hud::CANCEL`). `None` for a card or an action no card of this module has.
pub fn press<'a>(card: &'a str, action: &'a str, cancel: &str) -> Option<Press<'a>> {
    let machine = card.strip_prefix(PREFIX)?;
    if action == cancel {
        return Some(Press::Cancel);
    }
    match action {
        "vnc" => Some(Press::Open { machine, vnc: true }),
        "dash" => Some(Press::Open { machine, vnc: false }),
        _ => {
            let (verb, service) = action.split_once('/')?;
            match verb {
                "tail" => Some(Press::Tail { machine, service }),
                "restart" => Some(Press::Restart { machine, service }),
                _ => None,
            }
        }
    }
}

fn button(id: String, label: String, style: Style, kind: Kind) -> Action {
    Action { id, label, style, kind }
}

/// The restart button of `t`: a confirmed shell action showing `shown`, or disabled with why.
fn restart(t: &Target, shown: Result<String, String>) -> Action {
    let name = &t.service.name;
    let kind = match shown {
        Ok(cmd) => Kind::Local { run: Do::Shell(cmd), reply: false },
        Err(reason) => Kind::Disabled { raw: Value::Null, reply: false, reason },
    };
    button(format!("restart/{name}"), format!("Restart {name}…"), Style::Destructive, kind)
}

/// Machine `m`'s card: its own buttons, then each service's of `targets` (with the shown
/// restart command, or why it cannot run). `None` when it has none.
pub fn card(m: &Machine, targets: &[(Target, Result<String, String>)]) -> Option<Card> {
    let plain = |id: &str, label: &str| button(id.into(), label.into(), Style::Default, Kind::Reply);
    let vnc = m.vnc.as_ref().map(|_| plain("vnc", "Screen Sharing"));
    let dash = m.dash.as_ref().map(|_| plain("dash", "Open Dash"));
    let services = targets.iter().flat_map(|(t, shown)| {
        let name = &t.service.name;
        let tail = t.service.log.as_ref().map(|_| plain(&format!("tail/{name}"), &format!("Tail {name}")));
        let restart = t.service.restart.then(|| restart(t, shown.clone()));
        tail.into_iter().chain(restart)
    });
    let actions: Vec<Action> = vnc.into_iter().chain(dash).chain(services).collect();
    if actions.is_empty() {
        return None;
    }
    let card = Card {
        v: 1,
        id: card_id(&m.name),
        title: format!("{} · actions", m.name),
        state: State::Open,
        thread: None,
        reply_to: None,
        blocks: vec![],
        actions,
    };
    Some(card)
}

#[cfg(test)]
mod tests;
