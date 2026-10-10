//! The message module's table, `messages`, and the only SQL that touches it.

use rusqlite::{Row, params};
use serde::Serialize;

use super::seen::Dismissal;
use crate::core::store::Store;

/// Append only.
pub const MIGRATIONS: &[&str] = &[
    "CREATE TABLE messages (id TEXT PRIMARY KEY, ts INTEGER NOT NULL, title TEXT, body TEXT NOT NULL, url TEXT, reply_to TEXT, context TEXT, pending INTEGER NOT NULL DEFAULT 0);",
    // Cards (plan flick-7da1): `card` is the normalized card JSON (`core::card::to_json`),
    // NULL for a text message; `remote` is 1 when it was posted over the network.
    "ALTER TABLE messages ADD COLUMN card TEXT; ALTER TABLE messages ADD COLUMN remote INTEGER NOT NULL DEFAULT 0;",
    // Chat (plan pl-75d3, flick-a7b0): `thread` groups a conversation (NULL: the HUD's
    // history), `role` is 'me' for what the user typed here (NULL: a peer's post), `state`
    // is NULL when done, 'partial' while a reply streams, 'failed' when an ask did not go
    // out. Cards stored before this keep the thread their JSON names.
    "ALTER TABLE messages ADD COLUMN thread TEXT; ALTER TABLE messages ADD COLUMN role TEXT; ALTER TABLE messages ADD COLUMN state TEXT; CREATE INDEX messages_thread_ts ON messages (thread, ts); UPDATE messages SET thread = json_extract(card, '$.thread') WHERE card IS NOT NULL AND json_valid(card);",
    // Unread posts, dismissals (`seen.rs`; flick-cb7d, flick-07cd): `dismissed` NULL|'user'|'timeout'.
    "ALTER TABLE messages ADD COLUMN unread INTEGER NOT NULL DEFAULT 0; ALTER TABLE messages ADD COLUMN dismissed TEXT;",
];

/// Who wrote a message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// A post: KOTA or another peer, or a script on this Mac (`role` NULL).
    #[default]
    Peer,
    /// Typed by the user in this Mac's chat (`'me'`).
    Me,
}

/// How far along a message is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Progress {
    /// Complete (`state` NULL).
    #[default]
    Done,
    /// A reply still streaming: `post --partial`; the same id is posted again until a post
    /// without `--partial` ends it.
    Partial,
    /// An ask that did not reach KOTA.
    Failed,
}

impl Role {
    fn column(self) -> Option<&'static str> {
        (self == Role::Me).then_some("me")
    }

    fn read(s: Option<&str>) -> Self {
        if s == Some("me") { Role::Me } else { Role::Peer }
    }
}

impl Progress {
    fn column(self) -> Option<&'static str> {
        match self {
            Progress::Done => None,
            Progress::Partial => Some("partial"),
            Progress::Failed => Some("failed"),
        }
    }

    /// Unknown values read as done.
    fn read(s: Option<&str>) -> Self {
        match s {
            Some("partial") => Progress::Partial,
            Some("failed") => Progress::Failed,
            _ => Progress::Done,
        }
    }
}

/// How much history `put_message` keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    /// Messages outside threads (`max_history`).
    pub history: usize,
    /// Messages per thread (`chat_history`).
    pub per_thread: usize,
    /// Threads (`chat_threads`), by their newest message.
    pub threads: usize,
}

/// One thread: its id, how many messages it holds, and its first and last times.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Thread {
    pub id: String,
    pub messages: i64,
    /// Unix seconds of its oldest and newest message.
    pub first_ts: i64,
    pub last_ts: i64,
    /// The body of its oldest message (the question that opened it, usually).
    pub first_body: String,
}

/// One posted message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Message {
    pub id: String,
    /// Unix seconds.
    pub ts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The id of the message (usually a pending one) this answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// The body of the message this answers, shown as "Re: …".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// A placeholder its reply replaces.
    pub pending: bool,
    /// A card's normalized JSON (`core::card::to_json`); `body` then holds its plain text.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "as_json")]
    pub card: Option<String>,
    /// Posted over the network (`Cx::remote`): a card from an untrusted origin.
    #[serde(skip_serializing_if = "is_false")]
    pub remote: bool,
    /// The conversation it belongs to; None for the HUD's own history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(skip_serializing_if = "is_peer")]
    pub role: Role,
    #[serde(skip_serializing_if = "is_done")]
    pub state: Progress,
    /// Not seen yet; how its card left the corner (`seen.rs`).
    #[serde(skip_serializing_if = "is_false")]
    pub unread: bool,
    #[serde(skip)]
    pub dismissed: Option<Dismissal>,
}

/// A stored card as its JSON object rather than a string.
#[expect(clippy::ref_option, reason = "serde's serialize_with passes &T")]
fn as_json<S: serde::Serializer>(card: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    let value = card.as_deref().and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok());
    value.serialize(s)
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if passes &T")]
fn is_false(b: &bool) -> bool {
    !*b
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if passes &T")]
fn is_peer(r: &Role) -> bool {
    *r == Role::Peer
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if passes &T")]
fn is_done(p: &Progress) -> bool {
    *p == Progress::Done
}

const COLUMNS: &str = "id, ts, title, body, url, reply_to, context, pending, card, remote, thread, role, state, unread, dismissed";
const VALUES: &str = "?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15";

fn row(r: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get(0)?,
        ts: r.get(1)?,
        title: r.get(2)?,
        body: r.get(3)?,
        url: r.get(4)?,
        reply_to: r.get(5)?,
        context: r.get(6)?,
        pending: r.get(7)?,
        card: r.get(8)?,
        remote: r.get(9)?,
        thread: r.get(10)?,
        role: Role::read(r.get::<_, Option<String>>(11)?.as_deref()),
        state: Progress::read(r.get::<_, Option<String>>(12)?.as_deref()),
        unread: r.get(13)?,
        dismissed: Dismissal::read(r.get::<_, Option<String>>(14)?.as_deref()),
    })
}

/// Message history on the shared store, newest first.
pub trait Messages {
    /// Save `m`, replacing a message with the same id, then trim its scope to `keep`: the
    /// newest `keep.history` unthreaded messages, or for a threaded one the newest
    /// `keep.per_thread` of its thread, which stays, and of the other threads the
    /// `keep.threads - 1` with the newest last message. A message that replaces a threaded or partial one keeps that one's `ts` and
    /// place (a stream stays where it started); `m.thread` None keeps the stored thread.
    /// Otherwise a replacement is new (`ts` as given, newest at that time), as before chat.
    fn put_message(&self, m: &Message, keep: Keep) -> Result<(), String>;
    /// At most `limit` messages, newest first.
    fn messages(&self, limit: usize) -> Vec<Message>;
    /// At most `limit` cards (messages with a card), newest first.
    fn cards(&self, limit: usize) -> Vec<Message>;
    /// At most `limit` unread messages (`seen.rs`), newest first.
    fn unread(&self, limit: usize) -> Vec<Message>;
    fn message(&self, id: &str) -> Option<Message>;
    /// Remove message `id` if it is pending, and return it.
    fn take_pending(&self, id: &str) -> Option<Message>;
    /// The newest `limit` messages of `thread`, oldest first (transcript order).
    fn thread(&self, thread: &str, limit: usize) -> Vec<Message>;
    /// At most `limit` threads, the one with the newest message first.
    fn threads(&self, limit: usize) -> Vec<Thread>;
    /// Move message `id` to time `ts` (a question sent again goes out now, flick-1947).
    fn restamp(&self, id: &str, ts: i64);
}

impl Messages for Store {
    fn put_message(&self, m: &Message, keep: Keep) -> Result<(), String> {
        let old = self.message(&m.id);
        let stays = old.as_ref().is_some_and(|o| o.thread.is_some() || o.state == Progress::Partial);
        let ts = old.as_ref().filter(|_| stays).map_or(m.ts, |o| o.ts);
        let thread = m.thread.clone().or_else(|| old.and_then(|o| o.thread));
        // Replacing in place keeps the rowid (the tiebreak of equal times); otherwise the
        // row is new, as `INSERT OR REPLACE` always made it.
        let verb = if stays { "INSERT" } else { "INSERT OR REPLACE" };
        let update = if stays { UPSERT } else { "" };
        let conn = self.conn();
        conn.execute(
            &format!("{verb} INTO messages ({COLUMNS}) VALUES ({VALUES}){update}"),
            params![
                m.id, ts, m.title, m.body, m.url, m.reply_to, m.context, m.pending, m.card, m.remote,
                thread, m.role.column(), m.state.column(), m.unread, m.dismissed.map(Dismissal::column)
            ],
        )
        .map_err(|e| format!("message: can't save: {e}"))?;
        let n = |k: usize| i64::try_from(k).unwrap_or(i64::MAX);
        let trimmed = match &thread {
            None => conn.execute(
                "DELETE FROM messages WHERE thread IS NULL AND rowid NOT IN (SELECT rowid FROM messages WHERE thread IS NULL ORDER BY ts DESC, rowid DESC LIMIT ?1)",
                [n(keep.history)],
            ),
            Some(t) => conn
                .execute(
                    "DELETE FROM messages WHERE thread = ?2 AND rowid NOT IN (SELECT rowid FROM messages WHERE thread = ?2 ORDER BY ts DESC, rowid DESC LIMIT ?1)",
                    params![n(keep.per_thread), t],
                )
                .and_then(|_| {
                    // This thread stays, whatever its times, with the newest others.
                    conn.execute(
                        "DELETE FROM messages WHERE thread IS NOT NULL AND thread <> ?2 AND thread NOT IN (SELECT thread FROM messages WHERE thread IS NOT NULL AND thread <> ?2 GROUP BY thread ORDER BY MAX(ts) DESC, MAX(rowid) DESC LIMIT ?1)",
                        params![n(keep.threads.saturating_sub(1)), t],
                    )
                }),
        };
        trimmed.map_err(|e| format!("message: can't trim history: {e}"))?;
        Ok(())
    }

    fn messages(&self, limit: usize) -> Vec<Message> {
        newest(self, "", limit)
    }

    fn cards(&self, limit: usize) -> Vec<Message> {
        newest(self, "WHERE card IS NOT NULL", limit)
    }

    fn unread(&self, limit: usize) -> Vec<Message> {
        newest(self, "WHERE unread", limit)
    }

    fn message(&self, id: &str) -> Option<Message> {
        let sql = format!("SELECT {COLUMNS} FROM messages WHERE id = ?1");
        self.conn().query_row(&sql, [id], row).ok()
    }

    fn take_pending(&self, id: &str) -> Option<Message> {
        let m = self.message(id).filter(|m| m.pending)?;
        let _ = self.conn().execute("DELETE FROM messages WHERE id = ?1", [id]);
        Some(m)
    }

    fn thread(&self, thread: &str, limit: usize) -> Vec<Message> {
        let sql = format!("SELECT {COLUMNS} FROM messages WHERE thread = ?1 ORDER BY ts DESC, rowid DESC LIMIT ?2");
        let Ok(mut stmt) = self.conn().prepare(&sql) else { return vec![] };
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut list: Vec<Message> =
            stmt.query_map(params![thread, limit], row).map(|rows| rows.flatten().collect()).unwrap_or_default();
        list.reverse();
        list
    }

    fn threads(&self, limit: usize) -> Vec<Thread> {
        let sql = "SELECT thread, COUNT(*), MIN(ts), MAX(ts), (SELECT body FROM messages f WHERE f.thread = m.thread ORDER BY ts, rowid LIMIT 1) FROM messages m WHERE thread IS NOT NULL GROUP BY thread ORDER BY MAX(ts) DESC, MAX(rowid) DESC LIMIT ?1";
        let Ok(mut stmt) = self.conn().prepare(sql) else { return vec![] };
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let thread = |r: &Row| {
            Ok(Thread {
                id: r.get(0)?,
                messages: r.get(1)?,
                first_ts: r.get(2)?,
                last_ts: r.get(3)?,
                first_body: r.get(4)?,
            })
        };
        stmt.query_map([limit], thread).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    fn restamp(&self, id: &str, ts: i64) {
        let _ = self.conn().execute("UPDATE messages SET ts = ?2 WHERE id = ?1", params![id, ts]);
    }
}

/// The `ON CONFLICT` clause of a replacement in place: every column but `ts` from the new
/// row (`put_message` already chose the `ts` and thread).
const UPSERT: &str = " ON CONFLICT(id) DO UPDATE SET title = excluded.title, body = excluded.body, url = excluded.url, reply_to = excluded.reply_to, context = excluded.context, pending = excluded.pending, card = excluded.card, remote = excluded.remote, thread = excluded.thread, role = excluded.role, state = excluded.state, unread = excluded.unread, dismissed = excluded.dismissed";

/// At most `limit` messages matching `filter` (a WHERE clause or ""), newest first.
fn newest(store: &Store, filter: &str, limit: usize) -> Vec<Message> {
    let sql = format!("SELECT {COLUMNS} FROM messages {filter} ORDER BY ts DESC, rowid DESC LIMIT ?1");
    let Ok(mut stmt) = store.conn().prepare(&sql) else { return vec![] };
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    stmt.query_map([limit], row).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests;
