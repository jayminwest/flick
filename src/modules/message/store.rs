//! The message module's table, `messages`, and the only SQL that touches it.

use rusqlite::{Row, params};
use serde::Serialize;

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

const COLUMNS: &str = "id, ts, title, body, url, reply_to, context, pending, card, remote, thread, role, state";
const VALUES: &str = "?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13";

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
    fn message(&self, id: &str) -> Option<Message>;
    /// Remove message `id` if it is pending, and return it.
    fn take_pending(&self, id: &str) -> Option<Message>;
    /// The newest `limit` messages of `thread`, oldest first (transcript order).
    fn thread(&self, thread: &str, limit: usize) -> Vec<Message>;
    /// At most `limit` threads, the one with the newest message first.
    fn threads(&self, limit: usize) -> Vec<Thread>;
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
                thread, m.role.column(), m.state.column()
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
}

/// The `ON CONFLICT` clause of a replacement in place: every column but `ts` from the new
/// row (`put_message` already chose the `ts` and thread).
const UPSERT: &str = " ON CONFLICT(id) DO UPDATE SET title = excluded.title, body = excluded.body, url = excluded.url, reply_to = excluded.reply_to, context = excluded.context, pending = excluded.pending, card = excluded.card, remote = excluded.remote, thread = excluded.thread, role = excluded.role, state = excluded.state";

/// At most `limit` messages matching `filter` (a WHERE clause or ""), newest first.
fn newest(store: &Store, filter: &str, limit: usize) -> Vec<Message> {
    let sql = format!("SELECT {COLUMNS} FROM messages {filter} ORDER BY ts DESC, rowid DESC LIMIT ?1");
    let Ok(mut stmt) = store.conn().prepare(&sql) else { return vec![] };
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    stmt.query_map([limit], row).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let s = Store::in_memory();
        s.migrate("message", MIGRATIONS).unwrap();
        s
    }

    fn keep(history: usize) -> Keep {
        Keep { history, per_thread: 3, threads: 2 }
    }

    fn msg(id: &str, ts: i64) -> Message {
        Message { id: id.into(), ts, body: format!("body {id}"), ..Message::default() }
    }

    #[test]
    fn history_is_newest_first_and_trimmed() {
        let s = store();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            s.put_message(&msg(id, 100 + i as i64), keep(2)).unwrap();
        }
        let ids: Vec<_> = s.messages(10).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["c", "b"]);
        assert_eq!(s.messages(1).len(), 1);
        assert!(s.message("a").is_none());
        // Same second: the later insert is newer.
        s.put_message(&msg("d", 102), keep(5)).unwrap();
        assert_eq!(s.messages(1)[0].id, "d");
    }

    #[test]
    fn only_pending_messages_are_taken() {
        let s = store();
        s.put_message(&Message { pending: true, ..msg("p", 1) }, keep(5)).unwrap();
        s.put_message(&msg("q", 2), keep(5)).unwrap();
        assert!(s.take_pending("q").is_none());
        assert_eq!(s.take_pending("p").map(|m| m.body), Some("body p".into()));
        assert!(s.message("p").is_none());
        assert!(s.take_pending("p").is_none());
        assert_eq!(s.message("q").map(|m| m.pending), Some(false));
    }

    #[test]
    fn cards_keep_their_json_and_origin() {
        let s = store();
        let card = Message { card: Some(r#"{"id":"c","title":"T"}"#.into()), remote: true, ..msg("c", 2) };
        s.put_message(&msg("t", 1), keep(5)).unwrap();
        s.put_message(&card, keep(5)).unwrap();
        assert_eq!(s.message("c"), Some(card.clone()));
        assert_eq!(s.cards(10), [card]);
        assert_eq!(s.messages(10).len(), 2);
        // `ls --json`: the card as an object, `remote` only when set.
        let json = serde_json::to_string(&s.messages(10)).unwrap();
        assert!(json.contains(r#""card":{"id":"c","title":"T"},"remote":true}"#), "{json}");
        assert!(json.ends_with(r#""pending":false}]"#), "{json}");
    }

    #[test]
    fn migration_2_keeps_old_rows_as_local_text_messages() {
        let s = Store::in_memory();
        s.migrate("message", &MIGRATIONS[..1]).unwrap();
        s.conn().execute_batch("INSERT INTO messages (id, ts, body) VALUES ('old', 1, 'hi')").unwrap();
        s.migrate("message", MIGRATIONS).unwrap();
        let old = s.message("old").unwrap();
        assert_eq!((old.card, old.remote, old.body.as_str()), (None, false, "hi"));
    }
    fn in_thread(id: &str, ts: i64, thread: &str) -> Message {
        Message { thread: Some(thread.into()), ..msg(id, ts) }
    }

    fn ids(list: &[Message]) -> Vec<&str> {
        list.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn migration_3_adds_chat_columns_and_threads_old_cards() {
        let s = Store::in_memory();
        s.migrate("message", &MIGRATIONS[..2]).unwrap();
        s.conn()
            .execute_batch(
                r#"INSERT INTO messages (id, ts, body) VALUES ('old', 1, 'hi');
                   INSERT INTO messages (id, ts, body, card) VALUES ('c', 2, 'C', '{"id":"c","title":"C","thread":"ops"}');
                   INSERT INTO messages (id, ts, body, card) VALUES ('bad', 3, 'B', 'not json');"#,
            )
            .unwrap();
        s.migrate("message", MIGRATIONS).unwrap();
        let old = s.message("old").unwrap();
        assert_eq!((old.thread, old.role, old.state), (None, Role::Peer, Progress::Done));
        assert_eq!(s.message("c").unwrap().thread.as_deref(), Some("ops"));
        assert_eq!(s.message("bad").unwrap().thread, None);
        assert_eq!(ids(&s.thread("ops", 10)), ["c"]);
    }

    #[test]
    fn chat_fields_round_trip_and_serialize_only_when_set() {
        let s = store();
        let me = Message { role: Role::Me, state: Progress::Failed, ..in_thread("q", 1, "t1") };
        let partial = Message { state: Progress::Partial, ..in_thread("a", 2, "t1") };
        s.put_message(&me, keep(5)).unwrap();
        s.put_message(&partial, keep(5)).unwrap();
        s.put_message(&msg("plain", 3), keep(5)).unwrap();
        assert_eq!(s.message("q"), Some(me));
        assert_eq!(s.message("a"), Some(partial));
        // Unknown values written by a later version read as the defaults.
        s.conn().execute_batch("UPDATE messages SET role = 'bot', state = 'x' WHERE id = 'q'").unwrap();
        let q = s.message("q").unwrap();
        assert_eq!((q.role, q.state), (Role::Peer, Progress::Done));
        let json = serde_json::to_string(&s.thread("t1", 10)).unwrap();
        assert!(json.ends_with(r#""pending":false,"thread":"t1","state":"partial"}]"#), "{json}");
        let json = serde_json::to_string(&s.message("plain")).unwrap();
        assert!(json.ends_with(r#""pending":false}"#), "{json}");
        s.put_message(&Message { role: Role::Me, ..in_thread("q", 1, "t1") }, keep(5)).unwrap();
        let json = serde_json::to_string(&s.message("q")).unwrap();
        assert!(json.ends_with(r#""thread":"t1","role":"me"}"#), "{json}");
    }

    #[test]
    fn a_threaded_or_partial_repost_keeps_its_time_and_place() {
        let s = store();
        s.put_message(&in_thread("q", 10, "t"), keep(5)).unwrap();
        s.put_message(&Message { state: Progress::Partial, ..in_thread("a", 11, "t") }, keep(5)).unwrap();
        s.put_message(&in_thread("card", 11, "t"), keep(5)).unwrap();
        // Streaming: `a` again at a later time, then its final post without a thread.
        s.put_message(&Message { state: Progress::Partial, body: "more".into(), ..in_thread("a", 20, "t") }, keep(5))
            .unwrap();
        s.put_message(&Message { body: "all".into(), ..msg("a", 30) }, keep(5)).unwrap();
        let a = s.message("a").unwrap();
        assert_eq!((a.ts, a.body.as_str(), a.thread.as_deref(), a.state), (11, "all", Some("t"), Progress::Done));
        assert_eq!(ids(&s.thread("t", 10)), ["q", "a", "card"], "same second: insert order");
        // An unthreaded partial stream keeps its time too, until it is done.
        s.put_message(&msg("x", 40), keep(5)).unwrap();
        s.put_message(&Message { state: Progress::Partial, ..msg("u", 41) }, keep(5)).unwrap();
        s.put_message(&msg("y", 42), keep(5)).unwrap();
        s.put_message(&msg("u", 43), keep(5)).unwrap();
        assert_eq!(ids(&s.messages(3)), ["y", "u", "x"]);
        assert_eq!(s.message("u").unwrap().ts, 41);
        // A done unthreaded message posted again is new, as before chat.
        s.put_message(&msg("u", 44), keep(5)).unwrap();
        assert_eq!(ids(&s.messages(2)), ["u", "y"]);
        // A post may move a message into another thread.
        s.put_message(&in_thread("card", 50, "t2"), keep(5)).unwrap();
        assert_eq!(ids(&s.thread("t2", 10)), ["card"]);
        assert_eq!(s.message("card").unwrap().ts, 11);
    }

    #[test]
    fn history_is_trimmed_per_scope() {
        let s = store();
        // per_thread 3, threads 2, history 2.
        for i in 0..5 {
            s.put_message(&in_thread(&format!("a{i}"), 100 + i, "ta"), keep(2)).unwrap();
        }
        assert_eq!(ids(&s.thread("ta", 10)), ["a2", "a3", "a4"]);
        assert_eq!(ids(&s.thread("ta", 2)), ["a3", "a4"]);
        for id in ["h1", "h2", "h3"] {
            s.put_message(&msg(id, 200), keep(2)).unwrap();
        }
        // Unthreaded posts trim only unthreaded history; chat never evicts it.
        assert_eq!(s.thread("ta", 10).len(), 3);
        s.put_message(&in_thread("b0", 300, "tb"), keep(2)).unwrap();
        s.put_message(&in_thread("c0", 301, "tc"), keep(2)).unwrap();
        let threads: Vec<_> = s.threads(10).into_iter().map(|t| t.id).collect();
        assert_eq!(threads, ["tc", "tb"], "ta had the oldest last message");
        assert!(s.message("a4").is_none());
        assert_eq!(ids(&s.messages(10)), ["c0", "b0", "h3", "h2"]);
        // The thread posted to stays even when its times are the oldest.
        s.put_message(&Message { state: Progress::Partial, ..in_thread("b1", 302, "tb") }, keep(2)).unwrap();
        s.put_message(&in_thread("old", 1, "tb"), keep(2)).unwrap();
        s.put_message(&in_thread("old", 1, "tb"), Keep { history: 2, per_thread: 9, threads: 1 }).unwrap();
        let threads: Vec<_> = s.threads(10).into_iter().map(|t| t.id).collect();
        assert_eq!(threads, ["tb"]);
    }

    #[test]
    fn threads_list_newest_activity_first() {
        let s = store();
        assert_eq!(s.threads(5), []);
        assert_eq!(s.thread("none", 5), []);
        s.put_message(&Message { role: Role::Me, body: "first?".into(), ..in_thread("q1", 10, "t1") }, keep(5)).unwrap();
        s.put_message(&in_thread("r1", 12, "t1"), keep(5)).unwrap();
        s.put_message(&in_thread("q2", 11, "t2"), keep(5)).unwrap();
        s.put_message(&msg("loose", 13), keep(5)).unwrap();
        let t = s.threads(5);
        let first = Thread { id: "t1".into(), messages: 2, first_ts: 10, last_ts: 12, first_body: "first?".into() };
        assert_eq!(t[0], first);
        assert_eq!(t.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t1", "t2"]);
        assert_eq!(s.threads(1).len(), 1);
        let json = serde_json::to_string(&s.threads(1)).unwrap();
        assert_eq!(json, r#"[{"id":"t1","messages":2,"first_ts":10,"last_ts":12,"first_body":"first?"}]"#);
    }
}
