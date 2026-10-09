//! A flick.db written by the code before store migrations (master at 0627a57): opening a
//! copy keeps every row and the exact table SQL, and opening it again changes nothing.

use std::path::PathBuf;

use rusqlite::Connection;

use crate::modules::{AppStore as Store, Clips};

const FIXTURE: &[u8] = include_bytes!("fixtures/pre-store-migrations.db");

/// A copy of the fixture in the temp dir, deleted on drop.
struct Copy(PathBuf);

impl Copy {
    fn new(name: &str) -> Copy {
        let path =
            std::env::temp_dir().join(format!("flick-fixture-{}-{name}.db", std::process::id()));
        std::fs::write(&path, FIXTURE).unwrap();
        Copy(path)
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn dump(path: &PathBuf) -> Vec<String> {
    let conn = Connection::open(path).unwrap();
    let mut out = vec![];
    for sql in [
        "SELECT name || ': ' || sql FROM sqlite_master WHERE name IN ('usage', 'clips') ORDER BY name",
        "SELECT id || ' ' || count || ' ' || last FROM usage ORDER BY id",
        "SELECT id || ' ' || text || ' ' || ts FROM clips ORDER BY id",
        "SELECT name || ' ' || seq FROM sqlite_sequence",
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        out.extend(stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap));
    }
    out
}

#[test]
fn pre_migration_db_is_adopted_without_change() {
    let db = Copy::new("adopt");
    let before = dump(&db.0);
    assert_eq!(before.len(), 2 + 3 + 3 + 1);

    let store = Store::open(&db.0).unwrap();
    assert_eq!(store.usage()["app:/Applications/Safari.app"].0, 2);
    let texts: Vec<String> = store.clips().into_iter().map(|c| c.text).collect();
    assert_eq!(texts, ["first clip", "third clip", "second\nclip"]);
    assert_eq!(store.version("core").unwrap(), 1);
    assert_eq!(store.version("clip").unwrap(), 1);
    drop(store);
    assert_eq!(dump(&db.0), before);

    drop(Store::open(&db.0).unwrap());
    assert_eq!(dump(&db.0), before);
}
