//! `flick message card <verb>`: cards are messages with structure (plan flick-7da1). A card
//! is posted as JSON (`core::card`), stored in `messages.card` (normalized) with its plain
//! text in `body`, so history, `ls` and search keep working, and shown in the corner HUD as
//! that plain text until the card renderer lands (flick-aa43, flick-fd93).
//!
//! - `post <json>` (usually `--stdin`): parse from the caller's origin (`Cx::remote`); a
//!   structural error is the verb's error and nothing shows or is stored. The same id
//!   replaces the card (`INSERT OR REPLACE`); `reply_to` takes a pending message as
//!   `message post --reply-to` does. Answers the id and one `warning: …` line per warning,
//!   or with `--json` `{"id","replaced","warnings"}`.
//! - `get <id>`: the stored normalized JSON. `ls [--limit n]`: `id  time  state  title`.
//! - `show <id>`: show it again. `dismiss <id>|--all`: remove its card (every card's).
//!
//! A card the user dismissed comes back only as `open` or `error`: a `done` or `pending`
//! update of it goes to history without showing (plan risk 10).

use serde::Serialize;

use super::store::{Message, Messages};
use super::{Inbox, text};
use crate::core::Cx;
use crate::core::card::{self, Card, Origin, State};
use crate::platform::hud::Options;

const USAGE: &str = "usage: flick message card post <json>|--stdin | get <id> | ls [--limit n] | show <id> | dismiss <id>|--all";

/// Most dismissed ids remembered; past it the set starts over (a forgotten id only means a
/// `done` update of it shows again).
const DISMISSED_MAX: usize = 256;

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

fn origin(remote: bool) -> Origin {
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

fn state_name(state: State) -> &'static str {
    match state {
        State::Open => "open",
        State::Pending => "pending",
        State::Done => "done",
        State::Error => "error",
    }
}

impl Inbox {
    /// How a card behaves in the HUD: one that waits on the user (open, an enabled action)
    /// stays until acted on or closed; others time out like a message.
    pub(super) fn card_options(&self, c: &Card) -> Options {
        let sticky = c.waits_on_user();
        Options {
            timeout_secs: if sticky { 0.0 } else { self.settings.timeout_secs as f64 },
            sound: self.settings.sound && c.state != State::Pending,
            sticky,
        }
    }

    pub(super) fn card_verb(&mut self, args: &[String], cx: &Cx) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["post", json] => self.post_card(json, cx),
            ["get", id] => Ok(card_row(id, cx)?.card.unwrap_or_default()),
            ["ls"] => self.list_cards(20, cx),
            ["ls", "--limit", n] => {
                self.list_cards(n.parse().map_err(|_| format!("--limit {n}: not a number"))?, cx)
            }
            ["show", id] => {
                let m = card_row(id, cx)?;
                self.dismissed.remove(&m.id);
                self.display(&m, true);
                Ok(format!("Showing {id}"))
            }
            ["dismiss", "--all"] => {
                let cards = cx.store.cards(self.settings.max_history);
                let shown = cards.iter().filter(|m| (self.env.dismiss)(&m.id)).count();
                for m in cards {
                    self.forget(m.id);
                }
                Ok(format!("Dismissed {shown} card{}", if shown == 1 { "" } else { "s" }))
            }
            ["dismiss", id] => {
                let m = card_row(id, cx)?;
                (self.env.dismiss)(&m.id);
                self.forget(m.id);
                Ok(format!("Dismissed {id}"))
            }
            _ => Err(USAGE.into()),
        }
    }

    /// Remember that the user's side dismissed card `id`.
    fn forget(&mut self, id: String) {
        if self.dismissed.len() >= DISMISSED_MAX {
            self.dismissed.clear();
        }
        self.dismissed.insert(id);
    }

    fn post_card(&mut self, json: &str, cx: &Cx) -> Result<String, String> {
        let parsed = card::parse(json, origin(cx.remote)).map_err(|e| format!("invalid card: {e}"))?;
        let c = &parsed.card;
        let existed = cx.store.message(&c.id).is_some();
        let (context, took) = super::quote(c.reply_to.as_deref(), cx);
        let m = Message {
            id: c.id.clone(),
            ts: (self.env.now)(),
            body: card::plain(c),
            reply_to: c.reply_to.clone(),
            context,
            card: Some(card::to_json(c)),
            remote: cx.remote,
            ..Message::default()
        };
        self.save(&m, took, cx)?;
        let silent = matches!(c.state, State::Done | State::Pending) && self.dismissed.contains(&m.id);
        if !silent {
            self.dismissed.remove(&m.id);
            self.display(&m, false);
        }
        let replaced = existed || took;
        if cx.json {
            let posted = Posted { id: &m.id, replaced, warnings: &parsed.warnings };
            return serde_json::to_string(&posted).map_err(|e| e.to_string());
        }
        let warnings = parsed.warnings.iter().map(|w| format!("\nwarning: {w}"));
        Ok(std::iter::once(m.id.clone()).chain(warnings).collect())
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
