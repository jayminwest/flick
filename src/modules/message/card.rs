//! `flick message card <verb>`: cards are messages with structure (plan flick-7da1). A card
//! is posted as JSON (`core::card`), stored in `messages.card` (normalized) with its plain
//! text in `body`, so history, `ls` and search keep working, and drawn in the corner HUD by
//! its card renderer with the module's press state (`dispatch.rs`).
//!
//! - `post <json>` (usually `--stdin`): parse from the caller's origin (`Cx::remote`); a
//!   structural error is the verb's error and nothing shows or is stored. The same id
//!   replaces the card (`INSERT OR REPLACE`) and clears its press state (pending, error,
//!   confirm); `reply_to` takes a pending message as
//!   `message post --reply-to` does. Answers the id and one `warning: …` line per warning,
//!   or with `--json` `{"id","replaced","warnings"}`.
//! - `get <id>`: the stored normalized JSON. `ls [--limit n]`: `id  time  state  title`.
//! - `show <id>`: show it again. `dismiss <id>|--all`: remove its card (every card's).
//! - `spec`: `docs/cards.md`, the KOTA-facing spec, built in (`SPEC`); peers may read it.
//! - `focus`: move the keyboard into the newest card, as `card_hotkey` does (denied over the
//!   network: it takes the keyboard).
//! - `press <id> <action> [values-json]`: press a button as the HUD does, through the same
//!   `dispatch::press` (local testing; denied over the network). Values default to the
//!   card's initial inputs. A `shell` action shows its confirm on the first press; a second
//!   `card press` of the same action is Run. `press <id> :cancel` is Cancel.
//!
//! A card in the thread the chat window shows draws in its transcript instead of the corner
//! (`chat::model::alert`, as text posts); a corner card of it that already shows redraws in
//! place. A card the user dismissed comes back only as `open` or `error`: a `done` or `pending`
//! update of it goes to history without showing (plan risk 10). A card that only timed out
//! is not dismissed for this: any update of it shows (flick-9bdb). A `pending` card has no
//! timeout, so KOTA's final update of a long job finds it still up.

use serde::Serialize;

use super::dispatch::{Pressed, Ui};
use super::seen::{Dismissal, Seen};
use super::store::{Message, Messages};
use super::{Inbox, text};
use crate::core::Cx;
use crate::core::card::{self, Card, Origin, State, VALUES_MAX};
use crate::platform::hud::Options;

const USAGE: &str = "usage: flick message card post <json>|--stdin | get <id> | ls [--limit n] | show <id> | dismiss <id>|--all | spec | press <id> <action> [values-json] | focus";

/// The card spec for KOTA (`message card spec`): `docs/cards.md`, compiled in so a peer reads
/// the contract of the Flick it talks to. A test parses every `json` block in it as a card.
pub const SPEC: &str = include_str!("../../../docs/cards.md");

/// The JSON answer of `card post`.
#[derive(Serialize)]
struct Posted<'a> {
    id: &'a str,
    /// A card with this id was already stored, or `reply_to` took a pending message.
    replaced: bool,
    warnings: &'a [String],
}

/// One `card ls --json` entry.
#[derive(Serialize)]
struct Listed {
    id: String,
    ts: i64,
    remote: bool,
    card: serde_json::Value,
}

/// The origin of a card stored with this `remote` column.
pub fn origin(remote: bool) -> Origin {
    if remote { Origin::Remote } else { Origin::Local }
}

/// The card stored in `m`, read back with the origin it was posted from.
pub fn stored(m: &Message) -> Option<Card> {
    let json = m.card.as_deref()?;
    card::parse(json, origin(m.remote)).ok().map(|p| p.card)
}

/// What the text card shows under the header: `plain(card)` without its title line.
pub fn body(c: &Card) -> String {
    let plain = card::plain(c);
    text::plain(plain.split_once('\n').map_or("", |(_, rest)| rest))
}

/// The stored card `id`.
fn card_row(id: &str, cx: &Cx) -> Result<Message, String> {
    cx.store.message(id).filter(|m| m.card.is_some()).ok_or_else(|| format!("No card {id}"))
}

pub fn state_name(state: State) -> &'static str {
    match state {
        State::Open => "open",
        State::Pending => "pending",
        State::Done => "done",
        State::Error => "error",
    }
}

impl Inbox {
    /// How a card behaves in the HUD: one that waits on the user (open, an enabled action)
    /// stays `card_timeout_secs` (0: until acted on or closed), one waiting on KOTA (after a
    /// press, or posted `pending`) stays until KOTA or the watchdog changes it, so its final
    /// update shows (flick-9bdb); one nobody asked for (`seen::unprompted_card`) stays
    /// `unprompted_timeout_secs`; others time out like a message.
    pub(super) fn card_options(&self, c: &Card, ui: &Ui) -> Options {
        let sticky = c.waits_on_user();
        let secs = if ui.busy() || c.state == State::Pending {
            0
        } else if sticky {
            self.settings.card_timeout_secs
        } else if super::seen::unprompted_card(c) {
            self.settings.unprompted_timeout_secs
        } else {
            self.settings.timeout_secs
        };
        Options {
            timeout_secs: secs as f64,
            sound: self.settings.sound && c.state != State::Pending,
            sticky,
        }
    }

    pub(super) fn card_verb(&mut self, args: &[String], cx: &Cx) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["post", json] => self.post_card(json, cx),
            ["spec"] => Ok(SPEC.into()),
            ["get", id] => Ok(card_row(id, cx)?.card.unwrap_or_default()),
            ["ls"] => self.list_cards(20, cx),
            ["ls", "--limit", n] => {
                self.list_cards(n.parse().map_err(|_| format!("--limit {n}: not a number"))?, cx)
            }
            ["show", id] => {
                let m = card_row(id, cx)?;
                cx.store.set_dismissed(&m.id, None);
                self.display(&m, true);
                Ok(format!("Showing {id}"))
            }
            ["dismiss", "--all"] => {
                let cards = cx.store.cards(self.settings.max_history);
                let shown = cards.iter().filter(|m| (self.env.dismiss)(&m.id)).count();
                for m in cards {
                    self.forget(&m.id, cx);
                }
                Ok(format!("Dismissed {shown} card{}", if shown == 1 { "" } else { "s" }))
            }
            ["focus"] if (self.env.focus)() => Ok("A card has the keyboard; Esc gives it back".into()),
            ["focus"] => Err("No card shows".into()),
            ["press", id, action, values @ ..] if values.len() <= 1 => {
                self.press_verb(id, action, values.first().copied(), cx)
            }
            ["dismiss", id] => {
                let m = card_row(id, cx)?;
                (self.env.dismiss)(&m.id);
                self.forget(&m.id, cx);
                Ok(format!("Dismissed {id}"))
            }
            _ => Err(USAGE.into()),
        }
    }

    /// Store that the user's side dismissed card `id` (which reads a post), and drop its
    /// press state.
    pub(super) fn forget(&mut self, id: &str, cx: &Cx) {
        self.ui.remove(id);
        cx.store.set_dismissed(id, Some(Dismissal::User));
    }

    /// Card `id` timed out: its press state goes as `forget` has it, but it still waits on
    /// the user (and a post stays unread).
    pub(super) fn expire(&mut self, id: &str, cx: &Cx) {
        self.ui.remove(id);
        cx.store.set_dismissed(id, Some(Dismissal::Timeout));
    }

    fn post_card(&mut self, json: &str, cx: &Cx) -> Result<String, String> {
        let mut parsed = card::parse(json, origin(cx.remote)).map_err(|e| format!("invalid card: {e}"))?;
        let own = parsed.card.thread.take();
        parsed.card.thread = super::thread::inherit(own, &parsed.card.id, parsed.card.reply_to.as_deref(), cx);
        let c = &parsed.card;
        let old = cx.store.message(&c.id).map(|m| m.dismissed);
        let existed = old.is_some();
        // Only the user's dismissal silences an update; a card that timed out shows it.
        let silent = matches!(c.state, State::Done | State::Pending) && old == Some(Some(Dismissal::User));
        let (context, took) = super::quote(c.reply_to.as_deref(), cx);
        let m = Message {
            id: c.id.clone(),
            ts: (self.env.now)(),
            body: card::plain(c),
            reply_to: c.reply_to.clone(),
            context,
            card: Some(card::to_json(c)),
            remote: cx.remote,
            thread: c.thread.clone(),
            dismissed: old.flatten().filter(|_| silent),
            ..Message::default()
        };
        // A card nobody asked for is unread unless its update goes to history silently.
        let m = Message { unread: !silent && super::seen::unprompted(&m), ..m };
        self.save(&m, took, cx)?;
        self.ui.remove(&m.id);
        if !silent {
            if super::chat::model::alert(&m, !existed, self.chat_showing()).hud {
                self.display(&m, false);
            } else {
                // The chat window shows its thread: the card is there, not in the corner, and
                // a corner card of it from before only redraws in place.
                self.redraw(&m.id, cx);
            }
        }
        let replaced = existed || took;
        if cx.json {
            let posted = Posted { id: &m.id, replaced, warnings: &parsed.warnings };
            return serde_json::to_string(&posted).map_err(|e| e.to_string());
        }
        let warnings = parsed.warnings.iter().map(|w| format!("\nwarning: {w}"));
        Ok(std::iter::once(m.id.clone()).chain(warnings).collect())
    }

    /// `card press`: what the press did, in words.
    fn press_verb(&mut self, id: &str, action: &str, values: Option<&str>, cx: &Cx) -> Result<String, String> {
        let c = stored(&card_row(id, cx)?).ok_or(format!("No card {id}"))?;
        let values = match values {
            None => card::action::values_json(&c.inputs())?,
            Some(v) if !serde_json::from_str::<serde_json::Value>(v).is_ok_and(|v| v.is_object()) => {
                return Err("values must be a JSON object".into());
            }
            Some(v) if v.chars().count() > VALUES_MAX => return Err(format!("values over {VALUES_MAX} chars")),
            Some(v) => v.to_string(),
        };
        Ok(match self.press(id, action, values, cx)? {
            Pressed::Sent => format!("Sent {action} to KOTA; the card waits for its update"),
            Pressed::Confirm(cmd) => format!("Confirm on the card: {cmd}\nPress {action} again to run it"),
            Pressed::Cancelled => "Cancelled".into(),
            Pressed::Running(label) => format!("Running {label}; the result shows on the card"),
            Pressed::Did(line) => line,
            Pressed::Closed => format!("Closed {id}"),
        })
    }

    fn list_cards(&self, limit: usize, cx: &Cx) -> Result<String, String> {
        let list = cx.store.cards(limit);
        if cx.json {
            let listed: Vec<Listed> = list
                .into_iter()
                .map(|m| Listed {
                    card: m.card.as_deref().and_then(|c| serde_json::from_str(c).ok()).unwrap_or_default(),
                    id: m.id,
                    ts: m.ts,
                    remote: m.remote,
                })
                .collect();
            return serde_json::to_string(&listed).map_err(|e| e.to_string());
        }
        let now = (self.env.now)();
        let line = |m: &Message| {
            let time = text::stamp(m.ts, now, (self.env.utc_offset)(m.ts));
            let (state, title) = stored(m).map_or(("?", String::new()), |c| (state_name(c.state), c.title));
            format!("{}\t{time}\t{state}\t{}", m.id, text::preview(&title, 100))
        };
        Ok(list.iter().map(line).collect::<Vec<_>>().join("\n"))
    }
}
