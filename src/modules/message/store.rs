//! The message module's table, `messages`, and the only SQL that touches it.

use rusqlite::{Row, params};
use serde::Serialize;

use crate::core::store::Store;

/// Append only.
pub const MIGRATIONS: &[&str] = &[
    "CREATE TABLE messages (id TEXT PRIMARY KEY, ts INTEGER NOT NULL, title TEXT, body TEXT NOT NULL, url TEXT, reply_to TEXT, context TEXT, pending INTEGER NOT NULL DEFAULT 0);",
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
}

const COLUMNS: &str = "id, ts, title, body, url, reply_to, context, pending";

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
    })
}

/// Message history on the shared store, newest first.
pub trait Messages {
    /// Save `m`, replacing a message with the same id, then keep only the newest `keep`.
    fn put_message(&self, m: &Message, keep: usize) -> Result<(), String>;
    /// At most `limit` messages, newest first.
    fn messages(&self, limit: usize) -> Vec<Message>;
    fn message(&self, id: &str) -> Option<Message>;
    /// Remove message `id` if it is pending, and return it.
    fn take_pending(&self, id: &str) -> Option<Message>;
}

impl Messages for Store {
    fn put_message(&self, m: &Message, keep: usize) -> Result<(), String> {
        let conn = self.conn();
        conn.execute(
            &format!("INSERT OR REPLACE INTO messages ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"),
            params![m.id, m.ts, m.title, m.body, m.url, m.reply_to, m.context, m.pending],
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
        let sql = format!("SELECT {COLUMNS} FROM messages ORDER BY ts DESC, rowid DESC LIMIT ?1");
        let Ok(mut stmt) = self.conn().prepare(&sql) else { return vec![] };
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        stmt.query_map([limit], row).map(|rows| rows.flatten().collect()).unwrap_or_default()
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
}
