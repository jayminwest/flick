//! The activity module's tables, `activity_spans` and `activity_state`, and the only SQL
//! that touches them.

use rusqlite::params;

use crate::core::track::{Span, Subject};
use crate::store::Store;

/// Step 1 holds the `task` column for the tasks plan (flick-86be), so that plan needs no
/// migration on this table. Append only.
pub const MIGRATIONS: &[&str] = &["CREATE TABLE activity_spans (
        id INTEGER PRIMARY KEY, start INTEGER NOT NULL, end INTEGER NOT NULL,
        app TEXT NOT NULL, name TEXT NOT NULL, title TEXT, task INTEGER);
    CREATE INDEX activity_spans_start ON activity_spans (start);
    CREATE TABLE activity_state (key TEXT PRIMARY KEY, value TEXT);"];

/// `activity_state` key of the recording flag ("1" on; anything else or missing is off).
const RECORDING: &str = "recording";
/// `activity_state` key of the open span's row id.
const OPEN: &str = "open";


/// Activity spans and state on the shared store. Writes ignore SQL errors: a lost span
/// must never break the event loop.
pub trait Spans {
    /// Insert a span `at..at` for `subject`; its row id.
    fn span_open(&self, subject: &Subject, at: i64) -> Option<i64>;
    /// Move span `id`'s end to `at`.
    fn span_end(&self, id: i64, at: i64);
    fn span_delete(&self, id: i64);
    /// Spans that overlap `from..to`, oldest first.
    fn spans(&self, from: i64, to: i64) -> Vec<Span<Subject>>;
    fn recording(&self) -> bool;
    fn set_recording(&self, on: bool);
    /// The row id of the span stored as open.
    #[cfg(test)]
    fn open_span(&self) -> Option<i64>;
    fn set_open_span(&self, id: Option<i64>);
    /// Delete spans that end after `ts`; how many.
    fn forget_since(&self, ts: i64) -> usize;
    /// Delete spans of app `app` (bundle id or name); how many.
    fn forget_app(&self, app: &str) -> usize;
    /// Empty both tables, then VACUUM so the rows leave the file; how many spans.
    fn forget_all(&self) -> usize;
}

impl Spans for Store {
    fn span_open(&self, s: &Subject, at: i64) -> Option<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO activity_spans (start, end, app, name, title, task)
             VALUES (?1, ?1, ?2, ?3, ?4, ?5)",
            params![at, s.app, s.name, s.title, s.task],
        )
        .ok()?;
        Some(conn.last_insert_rowid())
    }

    fn span_end(&self, id: i64, at: i64) {
        let _ = self
            .conn()
            .execute("UPDATE activity_spans SET end = max(start, ?2) WHERE id = ?1", [id, at]);
    }

    fn span_delete(&self, id: i64) {
        let _ = self.conn().execute("DELETE FROM activity_spans WHERE id = ?1", [id]);
    }

    fn spans(&self, from: i64, to: i64) -> Vec<Span<Subject>> {
        let Ok(mut stmt) = self.conn().prepare(
            "SELECT start, end, app, name, title, task FROM activity_spans
             WHERE end > ?1 AND start < ?2 ORDER BY start, id",
        ) else {
            return vec![];
        };
        stmt.query_map([from, to], |r| {
            Ok(Span {
                start: r.get(0)?,
                end: r.get(1)?,
                subject: Subject {
                    app: r.get(2)?,
                    name: r.get(3)?,
                    title: r.get(4)?,
                    task: r.get(5)?,
                },
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    fn recording(&self) -> bool {
        state(self, RECORDING).as_deref() == Some("1")
    }

    fn set_recording(&self, on: bool) {
        set_state(self, RECORDING, Some(if on { "1" } else { "0" }));
    }

    #[cfg(test)]
    fn open_span(&self) -> Option<i64> {
        state(self, OPEN)?.parse().ok()
    }

    fn set_open_span(&self, id: Option<i64>) {
        set_state(self, OPEN, id.map(|id| id.to_string()).as_deref());
    }

    fn forget_since(&self, ts: i64) -> usize {
        self.conn().execute("DELETE FROM activity_spans WHERE end > ?1", [ts]).unwrap_or(0)
    }

    fn forget_app(&self, app: &str) -> usize {
        self.conn()
            .execute("DELETE FROM activity_spans WHERE app = ?1 OR name = ?1", [app])
            .unwrap_or(0)
    }

    fn forget_all(&self) -> usize {
        let conn = self.conn();
        let n = conn.execute("DELETE FROM activity_spans", []).unwrap_or(0);
        let _ = conn.execute("DELETE FROM activity_state", []);
        let _ = conn.execute_batch("VACUUM");
        n
    }
}

fn state(store: &Store, key: &str) -> Option<String> {
    store
        .conn()
        .query_row("SELECT value FROM activity_state WHERE key = ?1", [key], |r| r.get(0))
        .ok()
        .flatten()
}

fn set_state(store: &Store, key: &str, value: Option<&str>) {
    let conn = store.conn();
    let _ = match value {
        Some(v) => conn.execute(
            "INSERT INTO activity_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            [key, v],
        ),
        None => conn.execute("DELETE FROM activity_state WHERE key = ?1", [key]),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let s = Store::in_memory();
        s.migrate("activity", MIGRATIONS).unwrap();
        s
    }

    #[test]
    fn spans_round_trip_and_overlap() {
        let s = store();
        let a = Subject::new("com.a", "A", Some("doc"), Some(7));
        let id = s.span_open(&a, 100).unwrap();
        s.span_end(id, 160);
        let b = s.span_open(&Subject::new("com.b", "B", None, None), 200).unwrap();
        s.span_end(b, 260);
        let rows = s.spans(150, 210);
        assert_eq!(rows.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(), [(100, 160), (200, 260)]);
        assert_eq!(rows[0].subject, a);
        assert!(s.spans(260, 300).is_empty() && s.spans(0, 100).is_empty());
        s.span_end(id, 50); // never before its start
        assert_eq!(s.spans(0, 1000)[0].end, 100);
        s.span_delete(id);
        assert_eq!(s.spans(0, 1000).len(), 1);
    }

    #[test]
    fn state_is_off_until_set() {
        let s = store();
        assert!(!s.recording() && s.open_span().is_none());
        s.set_recording(true);
        s.set_open_span(Some(4));
        assert!(s.recording());
        assert_eq!(s.open_span(), Some(4));
        s.set_recording(false);
        s.set_open_span(None);
        assert!(!s.recording() && s.open_span().is_none());
        // Without the tables everything reads as empty and writes do nothing.
        let bare = Store::in_memory();
        bare.set_recording(true);
        assert!(!bare.recording() && bare.spans(0, 9).is_empty());
        assert!(bare.span_open(&Subject::new("a", "a", None, None), 1).is_none());
    }

    #[test]
    fn forget_deletes_rows() {
        let s = store();
        for (app, start) in [("com.a", 0), ("com.b", 100), ("com.a", 200)] {
            let id = s.span_open(&Subject::new(app, "Name", None, None), start).unwrap();
            s.span_end(id, start + 50);
        }
        assert_eq!(s.forget_since(200), 1);
        assert_eq!(s.forget_app("com.b"), 1);
        s.set_recording(true);
        assert_eq!(s.forget_all(), 1);
        assert!(s.spans(0, 1000).is_empty() && !s.recording());
    }
}
