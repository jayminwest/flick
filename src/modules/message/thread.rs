//! `flick message threads` and `flick message thread <t>`: read the chat threads in the
//! store (plan pl-75d3). A thread is the `thread` column: `post --thread`, a card's
//! `thread`. Read-only, so peers may call both.
//!
//! - `threads [--limit n]`: `id  time  n messages  first body`, newest activity first
//!   (default `chat_threads`); `--json`: `[{id,messages,first_ts,last_ts,first_body}]`.
//! - `thread <t> [--limit n]`: the newest n (default `chat_history`) messages of `t`, oldest
//!   first, as `ls` prints them; `--json`: the records. An empty or unknown thread is an
//!   error, `No thread <t>`.
//!
//! `inherit` (flick-3d84): a post or card without a thread of its own goes in the thread its
//! id is already stored in, else in that of the message it replies to, so KOTA's
//! `message post --reply-to <req>` answers a chat question in its thread without `--thread`.

use super::store::Messages;
use super::{Inbox, text};
use crate::core::Cx;

const USAGE: &str = "usage: flick message threads [--limit n] | thread <t> [--limit n]";

/// `--limit n`, or `default` without it.
fn limit(rest: &[&str], default: usize) -> Result<usize, String> {
    match rest {
        [] => Ok(default),
        ["--limit", n] => n.parse().map_err(|_| format!("--limit {n}: not a number")),
        _ => Err(USAGE.into()),
    }
}

impl Inbox {
    /// `args` starts with `thread` or `threads`.
    pub(super) fn thread_verb(&self, args: &[String], cx: &Cx) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let now = (self.env.now)();
        match words.as_slice() {
            ["threads", rest @ ..] => {
                let list = cx.store.threads(limit(rest, self.settings.chat_threads)?);
                if cx.json {
                    return serde_json::to_string(&list).map_err(|e| e.to_string());
                }
                let line = |t: &super::store::Thread| {
                    let time = text::stamp(t.last_ts, now, (self.env.utc_offset)(t.last_ts));
                    let n = t.messages;
                    let first = text::preview(&text::plain(&t.first_body), 80);
                    format!("{}\t{time}\t{n} message{}\t{first}", t.id, if n == 1 { "" } else { "s" })
                };
                Ok(list.iter().map(line).collect::<Vec<_>>().join("\n"))
            }
            ["thread", t, rest @ ..] => {
                let n = limit(rest, self.settings.chat_history)?;
                if !crate::core::card::valid_id(t) {
                    return Err(format!("{t}: a thread id is 1-64 of A-Z a-z 0-9 . _ -"));
                }
                let list = cx.store.thread(t, n);
                if list.is_empty() && cx.store.thread(t, 1).is_empty() {
                    return Err(format!("No thread {t}"));
                }
                if cx.json {
                    return serde_json::to_string(&list).map_err(|e| e.to_string());
                }
                Ok(self.ls(&list, now))
            }
            _ => Err(USAGE.into()),
        }
    }
}

/// The thread of a post with id `id`: `own` (`--thread`, a card's `thread`), else the thread
/// `id` is already stored in, else the thread of the message `reply_to` names, if any.
/// Read before `quote` takes a pending `reply_to` out of the store.
pub(super) fn inherit(own: Option<String>, id: &str, reply_to: Option<&str>, cx: &Cx) -> Option<String> {
    own.or_else(|| cx.store.message(id)?.thread).or_else(|| cx.store.message(reply_to?)?.thread)
}
