//! The clipboard module's table, `clips`, and the only SQL that touches it.

use rusqlite::params;

use crate::core::store::{Store, now};

const MAX_CLIPS: i64 = 500;

/// Step 1 adopts the `clips` table of a pre-versioning flick.db: the same SQL with
/// `IF NOT EXISTS`, so an existing table and its rows stay as they are. Append only.
pub const MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS clips (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL UNIQUE, ts INTEGER NOT NULL);",
];

pub struct Clip {
    pub id: i64,
    pub text: String,
    pub ts: i64,
}

/// Clip history on the shared store.
pub trait Clips {
    /// Add a clip, or move an existing identical clip to the top.
    fn add_clip(&self, text: &str);
    /// Clips, newest first.
    fn clips(&self) -> Vec<Clip>;
    fn clip_text(&self, id: i64) -> Option<String>;
}

impl Clips for Store {
    fn add_clip(&self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let conn = self.conn();
        // Re-insert so the clip gets the newest id; ids give the order.
        let _ = conn.execute("DELETE FROM clips WHERE text = ?1", [text]);
        let _ = conn.execute("INSERT INTO clips (text, ts) VALUES (?1, ?2)", params![text, now()]);
        let _ = conn.execute(
            "DELETE FROM clips WHERE id NOT IN (SELECT id FROM clips ORDER BY id DESC LIMIT ?1)",
            params![MAX_CLIPS],
        );
    }

    fn clips(&self) -> Vec<Clip> {
        let Ok(mut stmt) = self.conn().prepare("SELECT id, text, ts FROM clips ORDER BY id DESC")
        else {
            return vec![];
        };
        stmt.query_map([], |r| Ok(Clip { id: r.get(0)?, text: r.get(1)?, ts: r.get(2)? }))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    fn clip_text(&self, id: i64) -> Option<String> {
        self.conn().query_row("SELECT text FROM clips WHERE id = ?1", [id], |r| r.get(0)).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_dedupe_and_skip_blank() {
        let s = Store::in_memory();
        s.migrate("clip", MIGRATIONS).unwrap();
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
