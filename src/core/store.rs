//! flick.db: one shared SQLite connection. Each owner (core, or a module by its id)
//! declares an ordered list of migrations; `schema_versions` records how many of them ran.
//! An owner touches only its own tables. Core owns `usage` (frecency); a module's SQL
//! lives in that module.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::Usage;

/// Core's migrations. Step 1 adopts the `usage` table of a pre-versioning flick.db: the same
/// SQL with `IF NOT EXISTS`, so an existing table and its rows stay as they are.
const CORE_MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS usage (id TEXT PRIMARY KEY, count INTEGER NOT NULL, last INTEGER NOT NULL);",
];

pub struct Store {
    conn: Connection,
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Store {
    /// Open `path` and run core's migrations. Modules migrate their own tables with
    /// `migrate`.
    pub fn open(path: &Path) -> rusqlite::Result<Store> {
        Self::init(Connection::open(path)?)
    }

    #[expect(
        clippy::unwrap_used,
        reason = "an in-memory SQLite database with a fixed schema cannot fail to open"
    )]
    pub fn in_memory() -> Store {
        Self::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> rusqlite::Result<Store> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_versions (owner TEXT PRIMARY KEY, version INTEGER NOT NULL);",
        )?;
        let store = Store { conn };
        store.migrate("core", CORE_MIGRATIONS)?;
        Ok(store)
    }

    /// How many of `owner`'s migrations have run.
    pub fn version(&self, owner: &str) -> rusqlite::Result<usize> {
        let v: Option<i64> = self
            .conn
            .query_row("SELECT version FROM schema_versions WHERE owner = ?1", [owner], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(usize::try_from(v.unwrap_or(0)).unwrap_or(0))
    }

    /// Run `owner`'s migrations that have not run yet, in order, in one transaction.
    /// Migrations only append: never edit or reorder a released one. Running it again does
    /// nothing.
    pub fn migrate(&self, owner: &str, migrations: &[&str]) -> rusqlite::Result<()> {
        let done = self.version(owner)?;
        if done >= migrations.len() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        for sql in &migrations[done..] {
            tx.execute_batch(sql)?;
        }
        tx.execute(
            "INSERT INTO schema_versions (owner, version) VALUES (?1, ?2)
             ON CONFLICT(owner) DO UPDATE SET version = ?2",
            params![owner, migrations.len() as i64],
        )?;
        tx.commit()
    }

    /// The shared connection, for an owner's SQL against its own tables.
    pub fn conn(&self) -> &Connection {
        &self.conn
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
        if let Ok(mut stmt) = self.conn.prepare("SELECT id, count, last FROM usage")
            && let Ok(rows) =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))
        {
            out.extend(rows.flatten());
        }
        out
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
    fn migrations_run_once_in_order_per_owner() {
        let s = Store::in_memory();
        assert_eq!(s.version("core").unwrap(), 1);
        assert_eq!(s.version("toy").unwrap(), 0);
        let v1 = ["CREATE TABLE toy (a INTEGER);"];
        s.migrate("toy", &v1).unwrap();
        s.migrate("toy", &v1).unwrap();
        let v2 = [v1[0], "ALTER TABLE toy ADD COLUMN b TEXT;"];
        s.migrate("toy", &v2).unwrap();
        s.migrate("toy", &v2).unwrap();
        assert_eq!(s.version("toy").unwrap(), 2);
        s.conn().execute("INSERT INTO toy (a, b) VALUES (1, 'x')", []).unwrap();
    }

    #[test]
    fn a_failed_migration_leaves_no_trace() {
        let s = Store::in_memory();
        let bad = ["CREATE TABLE half (a INTEGER);", "NOT SQL;"];
        assert!(s.migrate("bad", &bad).is_err());
        assert_eq!(s.version("bad").unwrap(), 0);
        assert!(s.conn().prepare("SELECT a FROM half").is_err());
    }

    #[test]
    fn a_file_that_is_not_a_database_fails_to_open() {
        let path = std::env::temp_dir().join(format!("flick-store-{}.db", std::process::id()));
        std::fs::write(&path, b"not a sqlite database, just text that fills the header").unwrap();
        let opened = Store::open(&path);
        let _ = std::fs::remove_file(&path);
        assert!(opened.is_err());
    }

    #[test]
    fn usage_is_empty_without_its_table() {
        let s = Store::in_memory();
        s.record_use("a");
        s.conn().execute_batch("DROP TABLE usage;").unwrap();
        assert!(s.usage().is_empty());
    }
}
