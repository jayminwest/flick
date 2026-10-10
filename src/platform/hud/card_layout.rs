//! The pure side of the card renderer (`card_view`): what the module tells it beyond the card
//! (`CardUi`), the status a card shows, the confirm step, labels,
//! sizes, the button flow and the carry-over of live input values across a redraw. No
//! `AppKit` here, so all of it is unit-tested.

use crate::core::card::{Action, Card, Do, Input, Kind, State};

/// Inset of the content from the card's edges.
pub const PAD: f64 = 14.0;
/// The title line.
pub const TITLE_H: f64 = 18.0;
/// Space between blocks.
pub const GAP: f64 = 10.0;
/// One 11-point line (labels, the status line).
pub const SMALL_H: f64 = 15.0;
/// A push button.
pub const BUTTON_H: f64 = 24.0;
/// Space between buttons, and between button rows.
pub const BUTTON_GAP: f64 = 8.0;
/// Narrowest push button.
pub const BUTTON_MIN_W: f64 = 64.0;
/// A one-line text field.
pub const FIELD_H: f64 = 22.0;
/// A multiline text field (about three lines).
pub const MULTI_H: f64 = 58.0;
/// One radio button or checkbox row.
pub const CHOICE_ROW_H: f64 = 20.0;
/// A pop-up button.
pub const POPUP_H: f64 = 24.0;
/// A small progress bar.
pub const BAR_H: f64 = 12.0;
/// The spinner beside the pending line.
pub const SPINNER: f64 = 14.0;
/// Tallest text block; longer text is cut with `…`.
pub const MAX_TEXT_H: f64 = 240.0;
/// Tallest `kv` value; longer values are cut.
pub const MAX_VALUE_H: f64 = 48.0;
/// A single choice with at most this many options is radio buttons; more is a pop-up.
pub const RADIO_MAX: usize = 4;
/// The pop-up's first item when no option is selected.
pub const NO_CHOICE: &str = "Choose…";
/// The action id `on_press` gets when the user cancels a shell confirm. It is outside the
/// card id charset, so no real action can have it.
pub const CANCEL: &str = ":cancel";

/// Input values by input id, in card order (`Card::inputs`).
pub type Values = [(String, Input)];

/// What the module (which owns press state) tells the renderer beyond the card itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CardUi<'a> {
    /// A press went to KOTA and no update has come back: spinner, actions disabled.
    pub pending: bool,
    /// Shown in red under the body; actions stay enabled so the user can retry.
    pub error: Option<&'a str>,
    /// The id of a `shell` action waiting for confirmation: the card shows the exact command
    /// with Cancel and Run instead of its actions.
    pub confirm: Option<&'a str>,
    /// A muted line under the body: what a local action did ("Copied", a command's last
    /// output line), or, while `pending`, what runs instead of "Sent to KOTA…".
    pub note: Option<&'a str>,
}

/// How a status line looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Muted,
    Error,
}

/// What a card shows besides its blocks, from its state and the module's `CardUi`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// The SF Symbol beside the title.
    pub symbol: &'static str,
    /// A line under the body.
    pub line: Option<(Tone, String)>,
    /// A spinner before the line.
    pub spinner: bool,
    /// Whether actions and inputs take presses and edits.
    pub enabled: bool,
}

/// The status of a card in `state` with `ui`: pending wins (its line is the note, else "Sent
/// to KOTA…"), then an error from the module, then a note, then the card's own state.
pub fn status(state: State, ui: &CardUi) -> Status {
    let (symbol, line, spinner, enabled) = if ui.pending || state == State::Pending {
        let line = ui.note.filter(|_| ui.pending).unwrap_or("Sent to KOTA…");
        ("hourglass", Some((Tone::Muted, line.to_string())), true, false)
    } else if let Some(e) = ui.error {
        ("exclamationmark.triangle.fill", Some((Tone::Error, e.to_string())), false, true)
    } else {
        match state {
            State::Error => (
                "exclamationmark.triangle.fill",
                Some((Tone::Error, "KOTA reported an error".to_string())),
                false,
                true,
            ),
            State::Done => {
                ("checkmark.circle.fill", Some((Tone::Muted, "Done".into())), false, false)
            }
            State::Open | State::Pending => ("text.bubble.fill", None, false, true),
        }
    };
    let line = match ui.note {
        Some(n) if !spinner && ui.error.is_none() => Some((Tone::Muted, n.to_string())),
        _ => line,
    };
    Status { symbol, line, spinner, enabled }
}

/// The `shell` action waiting for confirmation and its exact command, if `ui.confirm` names
/// an enabled shell action on `card` and the card is not pending.
pub fn confirming<'c>(card: &'c Card, ui: &CardUi) -> Option<(&'c Action, &'c str)> {
    if !status(card.state, ui).enabled {
        return None;
    }
    let id = ui.confirm?;
    card.actions.iter().find(|a| a.id == id).and_then(|a| match &a.kind {
        Kind::Local { run: Do::Shell(cmd), .. } => Some((a, cmd.as_str())),
        _ => None,
    })
}

/// A button's tooltip: why it is disabled, or that it asks first.
pub fn tooltip(action: &Action) -> Option<String> {
    match &action.kind {
        Kind::Disabled { reason, .. } => Some(reason.clone()),
        Kind::Local { run: Do::Shell(cmd), .. } => Some(format!("Asks before running: {cmd}")),
        _ => None,
    }
}

/// A `list` row: `1. item` when ordered, else `• item`.
pub fn list_row(i: usize, item: &str, ordered: bool) -> String {
    if ordered { format!("{}. {item}", i + 1) } else { format!("• {item}") }
}

/// The line above a progress bar: the label and the percentage, or the label and `…` for an
/// indeterminate bar.
pub fn progress_text(value: Option<f64>, label: Option<&str>) -> String {
    let label = label.map(str::trim).filter(|l| !l.is_empty());
    match (value, label) {
        (Some(v), Some(l)) => format!("{l}  {}%", (v * 100.0).round()),
        (Some(v), None) => format!("{}%", (v * 100.0).round()),
        (None, Some(l)) => format!("{l}…"),
        (None, None) => "Working…".into(),
    }
}

/// The width of a `kv` key column from the keys' fitted widths: the widest plus a little,
/// at least 40 points and at most 40% of `inner` (longer keys are cut).
pub fn key_width(fitted: &[f64], inner: f64) -> f64 {
    let widest = fitted.iter().copied().fold(0.0, f64::max);
    (widest + 4.0).clamp(40.0, (inner * 0.4).max(40.0))
}

/// How a choice shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceStyle {
    Checkboxes,
    Radios,
    Popup,
}

/// Checkboxes for a `multi` choice, radio buttons for up to `RADIO_MAX` options, else a pop-up.
pub fn choice_style(multi: bool, options: usize) -> ChoiceStyle {
    if multi {
        ChoiceStyle::Checkboxes
    } else if options <= RADIO_MAX {
        ChoiceStyle::Radios
    } else {
        ChoiceStyle::Popup
    }
}

/// The height of a choice's controls (without its label).
pub fn choice_height(style: ChoiceStyle, options: usize) -> f64 {
    match style {
        ChoiceStyle::Popup => POPUP_H,
        ChoiceStyle::Checkboxes | ChoiceStyle::Radios => options as f64 * CHOICE_ROW_H,
    }
}

/// A push button's width from its fitted width: at least `BUTTON_MIN_W`, at most `inner`.
pub fn button_width(fitted: f64, inner: f64) -> f64 {
    (fitted + 12.0).max(BUTTON_MIN_W).min(inner)
}

/// Lay buttons of `widths` out right-aligned in rows of `inner` points, in order, wrapping
/// when a row is full. Returns each button's x (from the inner left edge) and row, and the
/// number of rows.
pub fn flow(widths: &[f64], inner: f64) -> (Vec<(f64, usize)>, usize) {
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for &w in widths {
        let w = w.min(inner);
        match rows.last_mut() {
            Some(row) if row.iter().sum::<f64>() + row.len() as f64 * BUTTON_GAP + w <= inner => {
                row.push(w);
            }
            _ => rows.push(vec![w]),
        }
    }
    let mut out = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let total = row.iter().sum::<f64>() + (row.len() - 1) as f64 * BUTTON_GAP;
        let mut x = inner - total;
        for w in row {
            out.push((x, r));
            x += w + BUTTON_GAP;
        }
    }
    (out, rows.len())
}

/// The height of `rows` rows of buttons.
pub fn buttons_height(rows: usize) -> f64 {
    if rows == 0 { 0.0 } else { rows as f64 * BUTTON_H + (rows - 1) as f64 * BUTTON_GAP }
}

/// The values to put in a redrawn card's inputs: an input keeps what the user entered while
/// the sender left its initial value alone (same id, same initial value as the last draw);
/// otherwise it takes the new initial value. `before` is the last draw's initial and live
/// values, in its input order.
pub fn carry(
    before: Option<(&Values, &Values)>,
    initial: Vec<(String, Input)>,
) -> Vec<(String, Input)> {
    let Some((old_initial, old_live)) = before else { return initial };
    initial
        .into_iter()
        .map(|(id, init)| {
            let same = old_initial.iter().any(|(i, v)| *i == id && *v == init);
            let live = old_live.iter().find(|(i, _)| *i == id).map(|(_, v)| v.clone());
            match live {
                Some(v) if same => (id, v),
                _ => (id, init),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
