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
];

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

const COLUMNS: &str = "id, ts, title, body, url, reply_to, context, pending, card, remote";

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
    })
}

/// Message history on the shared store, newest first.
pub trait Messages {
    /// Save `m`, replacing a message with the same id, then keep only the newest `keep`.
    fn put_message(&self, m: &Message, keep: usize) -> Result<(), String>;
    /// At most `limit` messages, newest first.
    fn messages(&self, limit: usize) -> Vec<Message>;
    /// At most `limit` cards (messages with a card), newest first.
    fn cards(&self, limit: usize) -> Vec<Message>;
    fn message(&self, id: &str) -> Option<Message>;
    /// Remove message `id` if it is pending, and return it.
    fn take_pending(&self, id: &str) -> Option<Message>;
}

impl Messages for Store {
    fn put_message(&self, m: &Message, keep: usize) -> Result<(), String> {
        let conn = self.conn();
        conn.execute(
            &format!("INSERT OR REPLACE INTO messages ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
            params![m.id, m.ts, m.title, m.body, m.url, m.reply_to, m.context, m.pending, m.card, m.remote],
        )
        .map_err(|e| format!("message: can't save: {e}"))?;
        let keep = i64::try_from(keep).unwrap_or(i64::MAX);
        conn.execute(
            "DELETE FROM messages WHERE rowid NOT IN (SELECT rowid FROM messages ORDER BY ts DESC, rowid DESC LIMIT ?1)",
            [keep],
        )
        .map_err(|e| format!("message: can't trim history: {e}"))?;
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
}

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

    fn msg(id: &str, ts: i64) -> Message {
        Message { id: id.into(), ts, body: format!("body {id}"), ..Message::default() }
    }

    #[test]
    fn history_is_newest_first_and_trimmed() {
        let s = store();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            s.put_message(&msg(id, 100 + i as i64), 2).unwrap();
        }
        let ids: Vec<_> = s.messages(10).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["c", "b"]);
        assert_eq!(s.messages(1).len(), 1);
        assert!(s.message("a").is_none());
        // Same second: the later insert is newer.
        s.put_message(&msg("d", 102), 5).unwrap();
        assert_eq!(s.messages(1)[0].id, "d");
    }

    #[test]
    fn only_pending_messages_are_taken() {
        let s = store();
        s.put_message(&Message { pending: true, ..msg("p", 1) }, 5).unwrap();
        s.put_message(&msg("q", 2), 5).unwrap();
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
        s.put_message(&msg("t", 1), 5).unwrap();
        s.put_message(&card, 5).unwrap();
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
}
