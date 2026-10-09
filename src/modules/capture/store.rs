//! The capture module's table, `capture_shots`, and the only SQL that touches it. The table
//! indexes screenshot files; it never holds image data, and trimming it never touches files.

use rusqlite::params;
use serde::Serialize;

use crate::store::Store;

/// Append only.
pub const MIGRATIONS: &[&str] = &[
    "CREATE TABLE capture_shots (id INTEGER PRIMARY KEY, path TEXT NOT NULL, kind TEXT NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL, taken INTEGER NOT NULL);
     CREATE INDEX capture_shots_taken ON capture_shots (taken);",
];

/// One recorded screenshot. Serializes as the `--json` shape of `capture ls` and `last`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    pub id: i64,
    pub path: String,
    /// What was captured: `area`, `window`, `screen`, `display` or `rect`; `annotated` for an
    /// edited copy.
    pub kind: String,
    pub width: u32,
    pub height: u32,
    /// Unix seconds.
    pub taken: i64,
}

/// Screenshot history on the shared store.
pub trait Shots {
    /// Record a screenshot, then keep only the newest `keep` rows. Returns the new row id.
    fn add_shot(&self, row: &Row, keep: u32) -> Option<i64>;
    /// At most `limit` rows, newest first.
    fn shots(&self, limit: u32) -> Vec<Row>;
    fn shot(&self, id: i64) -> Option<Row>;
    fn delete_shot(&self, id: i64);
}

impl Shots for Store {
    fn add_shot(&self, row: &Row, keep: u32) -> Option<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO capture_shots (path, kind, width, height, taken) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![row.path, row.kind, row.width, row.height, row.taken],
        )
        .ok()?;
        let id = conn.last_insert_rowid();
        let _ = conn.execute(
            "DELETE FROM capture_shots WHERE id NOT IN \
             (SELECT id FROM capture_shots ORDER BY taken DESC, id DESC LIMIT ?1)",
            [keep],
        );
        Some(id)
    }

    fn shots(&self, limit: u32) -> Vec<Row> {
        let sql = "SELECT id, path, kind, width, height, taken FROM capture_shots \
                   ORDER BY taken DESC, id DESC LIMIT ?1";
        let Ok(mut stmt) = self.conn().prepare(sql) else { return vec![] };
        stmt.query_map([limit], read)
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    fn shot(&self, id: i64) -> Option<Row> {
        let sql = "SELECT id, path, kind, width, height, taken FROM capture_shots WHERE id = ?1";
        self.conn().query_row(sql, [id], read).ok()
    }

    fn delete_shot(&self, id: i64) {
        let _ = self.conn().execute("DELETE FROM capture_shots WHERE id = ?1", [id]);
    }
}

fn read(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        path: r.get(1)?,
        kind: r.get(2)?,
        width: r.get(3)?,
        height: r.get(4)?,
        taken: r.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let s = Store::in_memory();
        s.migrate("capture", MIGRATIONS).unwrap();
        s
    }

    fn row(path: &str, taken: i64) -> Row {
        Row { id: 0, path: path.into(), kind: "area".into(), width: 800, height: 600, taken }
    }

    #[test]
    fn shots_list_newest_first_and_delete() {
        let s = store();
        let a = s.add_shot(&row("/a.png", 10), 200).unwrap();
        let b = s.add_shot(&row("/b.png", 20), 200).unwrap();
        let paths = |rows: Vec<Row>| rows.into_iter().map(|r| r.path).collect::<Vec<_>>();
        assert_eq!(paths(s.shots(10)), ["/b.png", "/a.png"]);
        assert_eq!(paths(s.shots(1)), ["/b.png"]);
        assert_eq!(s.shot(a), Some(Row { id: a, ..row("/a.png", 10) }));
        s.delete_shot(b);
        assert_eq!(s.shot(b), None);
        assert_eq!(paths(s.shots(10)), ["/a.png"]);
    }

    #[test]
    fn history_keeps_the_newest_rows() {
        let s = store();
        for t in 0..5 {
            s.add_shot(&row(&format!("/{t}.png"), t), 3);
        }
        let taken: Vec<i64> = s.shots(10).into_iter().map(|r| r.taken).collect();
        assert_eq!(taken, [4, 3, 2]);
    }

    #[test]
    fn a_missing_table_reads_as_empty() {
        let s = Store::in_memory();
        assert!(s.shots(10).is_empty());
        assert_eq!(s.shot(1), None);
        assert_eq!(s.add_shot(&row("/a.png", 1), 10), None);
    }
}
