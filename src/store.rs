//! SQLite persistence: item usage (frecency) and clipboard history.

use std::path::Path;

use rusqlite::{Connection, params};

use crate::search::Usage;

const MAX_CLIPS: i64 = 500;

pub struct Clip {
    pub id: i64,
    pub text: String,
    pub ts: i64,
}

pub struct Store {
    conn: Connection,
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Store> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Store {
        Self::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> rusqlite::Result<Store> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage (id TEXT PRIMARY KEY, count INTEGER NOT NULL, last INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS clips (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL UNIQUE, ts INTEGER NOT NULL);",
        )?;
        Ok(Store { conn })
    }

    pub fn record_use(&self, id: &str) {
        let _ = self.conn.execute(
            "INSERT INTO usage (id, count, last) VALUES (?1, 1, ?2)
             ON CONFLICT(id) DO UPDATE SET count = count + 1, last = ?2",
            params![id, now()],
        );
    }

    pub fn usage(&self) -> Usage {
        let mut out = Usage::new();
        if let Ok(mut stmt) = self.conn.prepare("SELECT id, count, last FROM usage") {
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))));
            if let Ok(rows) = rows {
                out.extend(rows.flatten());
            }
        }
        out
    }

    /// Add a clip, or move an existing identical clip to the top.
    pub fn add_clip(&self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        // Re-insert so the clip gets the newest id; ids give the order.
        let _ = self.conn.execute("DELETE FROM clips WHERE text = ?1", [text]);
        let _ = self.conn.execute("INSERT INTO clips (text, ts) VALUES (?1, ?2)", params![text, now()]);
        let _ = self.conn.execute(
            "DELETE FROM clips WHERE id NOT IN (SELECT id FROM clips ORDER BY id DESC LIMIT ?1)",
            params![MAX_CLIPS],
        );
    }

    /// Clips, newest first.
    pub fn clips(&self) -> Vec<Clip> {
        let Ok(mut stmt) = self.conn.prepare("SELECT id, text, ts FROM clips ORDER BY id DESC") else {
            return vec![];
        };
        stmt.query_map([], |r| Ok(Clip { id: r.get(0)?, text: r.get(1)?, ts: r.get(2)? }))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    pub fn clip_text(&self, id: i64) -> Option<String> {
        self.conn.query_row("SELECT text FROM clips WHERE id = ?1", [id], |r| r.get(0)).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_counts() {
        let s = Store::in_memory();
        s.record_use("a");
        s.record_use("a");
        assert_eq!(s.usage()["a"].0, 2);
    }

    #[test]
    fn clips_dedupe_and_skip_blank() {
        let s = Store::in_memory();
        s.add_clip("one");
        s.add_clip("two");
        s.add_clip("one");
        s.add_clip("   ");
        let clips = s.clips();
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].text, "one");
        assert_eq!(s.clip_text(clips[1].id).as_deref(), Some("two"));
    }
}
