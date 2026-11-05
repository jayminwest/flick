//! The tile module's table `tile_hidden` and the only SQL that touches it: every window `tile`
//! parked at a screen corner, with the frame to put it back at. A row is written before its
//! window is parked and deleted once the window is put back, so a Flick that was killed finds
//! the rows at its next start.

use rusqlite::{Connection, Row, params};

use crate::core::store::Store;
use crate::platform::Rect;
use crate::platform::winman::WinId;

/// Append only: existing flick.db files counted every released step.
pub const MIGRATIONS: &[&str] = &["CREATE TABLE tile_hidden (
        id INTEGER PRIMARY KEY, pid INTEGER NOT NULL, bundle_id TEXT,
        x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL)"];

/// A parked window: its window-server id (valid across a Flick restart), its app, and its
/// frame before it was parked.
#[derive(Clone, Debug, PartialEq)]
pub struct Parked {
    pub id: WinId,
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub frame: Rect,
}

/// `tile_hidden` on the shared store.
pub trait TileStore {
    /// Record windows about to be parked, replacing older rows of the same windows. `Err`
    /// writes none of them.
    fn parked_put(&self, rows: &[Parked]) -> Result<(), String>;
    /// Every recorded window, by id; empty on an SQL error.
    fn parked_all(&self) -> Vec<Parked>;
    /// Forget windows that were put back or are gone. Errors are ignored: a row left over
    /// only puts its window back at the next start.
    fn parked_delete(&self, ids: &[WinId]);
}

impl TileStore for Store {
    fn parked_put(&self, rows: &[Parked]) -> Result<(), String> {
        put(self.conn(), rows).map_err(|e| format!("tile: {e}"))
    }

    fn parked_all(&self) -> Vec<Parked> {
        all(self.conn()).unwrap_or_default()
    }

    fn parked_delete(&self, ids: &[WinId]) {
        for id in ids {
            let _ = self.conn().execute("DELETE FROM tile_hidden WHERE id = ?1", [id]);
        }
    }
}

const INSERT: &str = "INSERT OR REPLACE INTO tile_hidden VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

fn put(conn: &Connection, rows: &[Parked]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let mut insert = tx.prepare(INSERT)?;
    for r in rows {
        let f = r.frame;
        insert.execute(params![r.id, r.pid, r.bundle_id, f.x, f.y, f.w, f.h])?;
    }
    drop(insert);
    tx.commit()
}

fn all(conn: &Connection) -> rusqlite::Result<Vec<Parked>> {
    let mut select =
        conn.prepare("SELECT id, pid, bundle_id, x, y, w, h FROM tile_hidden ORDER BY id")?;
    select.query_map([], parked)?.collect()
}

fn parked(r: &Row) -> rusqlite::Result<Parked> {
    let frame = Rect { x: r.get(3)?, y: r.get(4)?, w: r.get(5)?, h: r.get(6)? };
    Ok(Parked { id: r.get(0)?, pid: r.get(1)?, bundle_id: r.get(2)?, frame })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: WinId, x: f64) -> Parked {
        Parked {
            id,
            pid: 7,
            bundle_id: Some("com.example.app".into()),
            frame: Rect { x, y: 25.0, w: 800.0, h: 600.0 },
        }
    }

    #[test]
    fn rows_round_trip_and_replace_by_id() {
        let store = Store::in_memory();
        store.migrate("tile", MIGRATIONS).unwrap();
        store.parked_put(&[row(3, 1.0), row(1, 2.0)]).unwrap();
        store.parked_put(&[Parked { bundle_id: None, ..row(3, 5.5) }]).unwrap();
        assert_eq!(store.parked_all(), [row(1, 2.0), Parked { bundle_id: None, ..row(3, 5.5) }]);
        store.parked_delete(&[1, 9]);
        assert_eq!(store.parked_all().len(), 1);
    }

    #[test]
    fn without_the_table_puts_fail_and_reads_are_empty() {
        let store = Store::in_memory();
        let err = store.parked_put(&[row(1, 0.0)]).unwrap_err();
        assert!(err.starts_with("tile: no such table: tile_hidden"), "{err}");
        assert!(store.parked_all().is_empty());
        store.parked_delete(&[1]);
    }
}
