//! `message ls [--unread] [--limit n]` and `message read <id>|--all` (flick-8ce3): the
//! history as text or JSON, and marking unread posts (`seen.rs`) read from a terminal or a
//! peer.
//!
//! - `ls` prints `id  time  [title: ]body[ (pending|partial|failed|unread)]`, newest first;
//!   `--unread` keeps only the unread posts (the newest `n` of them, not the unread among
//!   the newest `n`). `thread <t>` prints its rows the same way.
//! - `read` reads as opening the message list does: the post is no longer unread and its
//!   corner card closes. A post that is not unread is left alone. Answers `Read <id>` or
//!   `Read <n> posts`, with `--json` `{"read":[ids]}` (the ids that were unread). Peers may
//!   send it, as every `message` verb that takes no keyboard and runs no ssh.

use serde::Serialize;

use super::seen::Seen;
use super::store::{Message, Messages, Progress};
use super::{Inbox, text};
use crate::core::Cx;

const LS_USAGE: &str = "usage: flick message ls [--unread] [--limit n]";
const READ_USAGE: &str = "usage: flick message read <id>|--all";

/// The JSON answer of `read`.
#[derive(Serialize)]
struct Read<'a> {
    read: &'a [String],
}

impl Inbox {
    /// `ls` text: one line per message, `id  time  [title: ]body[ (pending|partial|failed|unread)]`.
    pub(super) fn ls(&self, list: &[Message], now: i64) -> String {
        let line = |m: &Message| {
            let time = text::stamp(m.ts, now, (self.env.utc_offset)(m.ts));
            let title = m.title.as_ref().map(|t| format!("{t}: ")).unwrap_or_default();
            let mark = match (m.pending, m.state) {
                (true, _) => " (pending)",
                (_, Progress::Partial) => " (partial)",
                (_, Progress::Failed) => " (failed)",
                (_, Progress::Done) if m.unread => " (unread)",
                (_, Progress::Done) => "",
            };
            let body = text::preview(&text::plain(&m.body), 100);
            format!("{}\t{time}\t{title}{body}{mark}", m.id)
        };
        list.iter().map(line).collect::<Vec<_>>().join("\n")
    }

    /// `ls [--unread] [--limit n]`, the flags in any order.
    pub(super) fn ls_verb(&self, mut rest: &[&str], cx: &Cx) -> Result<String, String> {
        let (mut limit, mut unread) = (20, false);
        loop {
            match rest {
                [] => break,
                ["--unread", more @ ..] => (unread, rest) = (true, more),
                ["--limit", n, more @ ..] => {
                    limit = n.parse().map_err(|_| format!("--limit {n}: not a number"))?;
                    rest = more;
                }
                _ => return Err(LS_USAGE.into()),
            }
        }
        let list = if unread { cx.store.unread(limit) } else { cx.store.messages(limit) };
        if cx.json {
            return serde_json::to_string(&list).map_err(|e| e.to_string());
        }
        Ok(self.ls(&list, (self.env.now)()))
    }

    /// `read <id>|--all`: mark it (every unread post) read and close its corner card.
    pub(super) fn read_verb(&self, rest: &[&str], cx: &Cx) -> Result<String, String> {
        let read = match rest {
            ["--all"] => cx.store.read_all(),
            [id] if cx.store.message(id).is_none() => return Err(format!("No message {id}")),
            [id] => cx.store.read_one(id).then(|| (*id).to_string()).into_iter().collect(),
            _ => return Err(READ_USAGE.into()),
        };
        for id in &read {
            (self.env.dismiss)(id);
        }
        if cx.json {
            return serde_json::to_string(&Read { read: &read }).map_err(|e| e.to_string());
        }
        Ok(match rest {
            ["--all"] => format!("Read {} post{}", read.len(), if read.len() == 1 { "" } else { "s" }),
            _ => format!("Read {}", rest.join(" ")),
        })
    }
}
