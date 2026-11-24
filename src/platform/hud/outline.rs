//! The text outline of a card, for a driven instance's scene (`crate::drive`): what a card
//! shows, one line per part, so an agent reads it without a screenshot. Pure; corner cards
//! keep theirs from their last draw, surface rows build theirs on request.

use super::TextCard;
use super::card_layout::{CANCEL, CardUi, confirming, status};
use crate::core::card::{self, Card, Input};

/// `card` drawn in state `ui`: its title, its blocks as `| ` lines, each input's initial
/// value, the status line, then its actions as `id "Label"` (`(off)` when it cannot be
/// pressed) or, in a shell confirm, the command with Run and Cancel.
pub fn card(card: &Card, ui: &CardUi) -> Vec<String> {
    let mut out = vec![format!("title: {}", card.title)];
    let plain = card::plain(card);
    let mut body: Vec<&str> = plain.lines().skip(1).collect();
    if !card.actions.is_empty() {
        // `plain` ends with the action labels; the actions line below names their ids.
        body.pop();
    }
    out.extend(body.iter().map(|l| format!("| {l}")));
    for (id, input) in card.inputs() {
        out.push(format!("input {id}: {}", value(&input)));
    }
    let status = status(card.state, ui);
    if let Some((_, line)) = &status.line {
        out.push(format!("status: {line}"));
    }
    if let Some((action, cmd)) = confirming(card, ui) {
        out.push(format!("confirm: {cmd}"));
        out.push(format!("actions: {} \"Run\", {CANCEL} \"Cancel\"", action.id));
    } else if !card.actions.is_empty() {
        let actions: Vec<String> = card
            .actions
            .iter()
            .map(|a| {
                let off = if status.enabled && a.enabled() { "" } else { " (off)" };
                format!("{} \"{}\"{off}", a.id, a.label)
            })
            .collect();
        out.push(format!("actions: {}", actions.join(", ")));
    }
    out
}

fn value(input: &Input) -> String {
    match input {
        Input::Text(text) => format!("\"{text}\""),
        Input::One(choice) => choice.clone().unwrap_or_default(),
        Input::Many(choices) => choices.join(", "),
    }
}

/// A text card: header and time, the context line, the body as `| ` lines, the link.
pub fn text(card: &TextCard) -> Vec<String> {
    let mut out = vec![format!("header: {}  {}", card.header, card.time).trim_end().to_string()];
    if !card.context.is_empty() {
        out.push(format!("context: {}", card.context));
    }
    out.extend(card.body.lines().map(|l| format!("| {l}")));
    if card.pending {
        out.push("status: waiting for a reply".into());
    }
    if let Some(link) = card.link {
        out.push(format!("link: {link}"));
    }
    out
}

#[cfg(test)]
mod tests;
