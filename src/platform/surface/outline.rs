//! The text outline of a surface, for a driven instance's scene (`crate::drive`): its
//! header, transcript rows, notice, chips and input, so an agent reads it without a
//! screenshot. Pure; `driven` gathers what the views hold.

use std::fmt::Write as _;

use super::Status;
use super::rows::{BubbleState, Owned, Side};
use crate::platform::hud;

/// What one surface holds.
#[derive(Clone, Copy, Debug)]
pub struct Outline<'a> {
    pub name: &'a str,
    pub shown: bool,
    /// A private surface shows only its name: its content never leaves it.
    pub private: bool,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub status: Status,
    pub rows: &'a [Owned],
    /// `None` while hidden.
    pub notice: Option<&'a str>,
    pub chips: &'a [String],
    pub input: &'a str,
}

/// `surface: <name> (shown|hidden[, private])`, then, for a shown surface that is not
/// private, its header, rows (bubbles, cards as `hud::outline`, dividers), notice, chips
/// and input, indented.
pub fn surface(o: &Outline) -> String {
    let shown = if o.shown { "shown" } else { "hidden" };
    let private = if o.private { ", private" } else { "" };
    let mut out = format!("surface: {} ({shown}{private})\n", o.name);
    if !o.shown || o.private {
        return out;
    }
    let status = match o.status {
        Status::Idle => "",
        Status::Busy => " (busy)",
        Status::Error => " (error)",
    };
    let subtitle =
        if o.subtitle.is_empty() { String::new() } else { format!(" — {}", o.subtitle) };
    let _ = writeln!(out, "  title: {}{subtitle}{status}", o.title);
    let _ = writeln!(out, "  rows: {}", o.rows.len());
    for row in o.rows {
        for line in self::row(row) {
            let _ = writeln!(out, "    {line}");
        }
    }
    if let Some(notice) = o.notice {
        let _ = writeln!(out, "  notice: {notice}");
    }
    if !o.chips.is_empty() {
        let _ = writeln!(out, "  chips: {}", o.chips.join(", "));
    }
    let _ = writeln!(out, "  input: \"{}\"", o.input);
    out
}

/// One transcript row: a header line, then what it shows indented.
fn row(row: &Owned) -> Vec<String> {
    match row {
        Owned::Bubble { side, header, time, md, state, .. } => {
            let side = match side {
                Side::Mine => "mine",
                Side::Theirs => "theirs",
                Side::System => "system",
            };
            let state = match state {
                BubbleState::Done => "",
                BubbleState::Streaming => " (streaming)",
                BubbleState::Pending => " (pending)",
                BubbleState::Failed => " (failed)",
            };
            let who: Vec<&str> =
                [header, time].into_iter().map(String::as_str).filter(|s| !s.is_empty()).collect();
            let who = if who.is_empty() { String::new() } else { format!(": {}", who.join("  ")) };
            let mut out = vec![format!("bubble {side}{who}{state}")];
            out.extend(md.lines().map(|l| format!("  | {l}")));
            out
        }
        Owned::Card { card, ui, .. } => {
            let mut out = vec![format!("card: {}", card.id)];
            out.extend(hud::card_outline(card, &ui.get()).into_iter().map(|l| format!("  {l}")));
            out
        }
        Owned::Divider { text } => vec![format!("divider: {text}")],
    }
}

#[cfg(test)]
mod tests;
