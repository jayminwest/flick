//! flick.db: the `usage` and `clips` tables, the 500-clip cap and clip dedupe.

use std::path::PathBuf;

use rusqlite::Connection;

use crate::modules::{AppStore as Store, Clips, IDS};

/// A database file in the temp dir, deleted on drop.
struct TempDb(PathBuf);

impl TempDb {
    fn new(name: &str) -> TempDb {
        let path =
            std::env::temp_dir().join(format!("flick-char-{}-{name}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        TempDb(path)
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn table_sql(path: &PathBuf) -> Vec<(String, String)> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare("SELECT name, sql FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
}

#[test]
fn schema_is_usage_and_clips() {
    let db = TempDb::new("schema");
    drop(Store::open(&db.0).unwrap());
    let known = ["clips", "schema_versions", "sqlite_sequence", "usage"];
    let (pinned, rest): (Vec<_>, Vec<_>) =
        table_sql(&db.0).into_iter().partition(|(name, _)| known.contains(&name.as_str()));
    // Any other table belongs to a module not pinned here, named after its id.
    let pinned_ids = ["app", "desktop", "switcher", "window", "quicklink", "builtin", "clip"];
    let new_ids: Vec<&str> = IDS.iter().copied().filter(|id| !pinned_ids.contains(id)).collect();
    for (name, _) in &rest {
        assert!(new_ids.iter().any(|id| name.starts_with(id)), "unexpected table {name}");
    }
    assert_eq!(
        pinned,
        [
            (
                "clips".to_string(),
                "CREATE TABLE clips (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL UNIQUE, ts INTEGER NOT NULL)".to_string()
            ),
            (
                "schema_versions".to_string(),
                "CREATE TABLE schema_versions (owner TEXT PRIMARY KEY, version INTEGER NOT NULL)".to_string()
            ),
            ("sqlite_sequence".to_string(), "CREATE TABLE sqlite_sequence(name,seq)".to_string()),
            (
                "usage".to_string(),
                "CREATE TABLE usage (id TEXT PRIMARY KEY, count INTEGER NOT NULL, last INTEGER NOT NULL)".to_string()
            ),
        ]
    );
}

#[test]
fn existing_rows_survive_open() {
    let db = TempDb::new("existing");
    {
        let conn = Connection::open(&db.0).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage (id TEXT PRIMARY KEY, count INTEGER NOT NULL, last INTEGER NOT NULL);
             CREATE TABLE clips (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL UNIQUE, ts INTEGER NOT NULL);
             INSERT INTO usage VALUES ('app:/Applications/Safari.app', 7, 1700000000);
             INSERT INTO usage VALUES ('builtin:Clipboard History', 2, 1700000100);
             INSERT INTO clips (id, text, ts) VALUES (41, 'older', 1700000000);
             INSERT INTO clips (id, text, ts) VALUES (42, 'newer', 1700000050);",
        )
        .unwrap();
    }
    let store = Store::open(&db.0).unwrap();
    let usage = store.usage();
    assert_eq!(usage.len(), 2);
    assert_eq!(usage["app:/Applications/Safari.app"], (7, 1_700_000_000));
    assert_eq!(usage["builtin:Clipboard History"], (2, 1_700_000_100));
    let clips: Vec<(i64, String, i64)> =
        store.clips().into_iter().map(|c| (c.id, c.text, c.ts)).collect();
    assert_eq!(
        clips,
        [(42, "newer".to_string(), 1_700_000_050), (41, "older".to_string(), 1_700_000_000)]
    );

    store.record_use("app:/Applications/Safari.app");
    assert_eq!(store.usage()["app:/Applications/Safari.app"].0, 8);
    store.add_clip("newest");
    assert_eq!(store.clips()[0].id, 43);
}

#[test]
fn record_use_counts_and_stamps_now() {
    let store = Store::in_memory();
    let before = crate::store::now();
    store.record_use("quicklink:Google");
    store.record_use("quicklink:Google");
    store.record_use("window:Left Half");
    let usage = store.usage();
    assert_eq!(usage.len(), 2);
    let (count, last) = usage["quicklink:Google"];
    assert_eq!(count, 2);
    assert!(last >= before && last <= crate::store::now());
    assert_eq!(usage["window:Left Half"].0, 1);
}

#[test]
fn clips_cap_at_500_newest_first() {
    let store = Store::in_memory();
    for i in 0..505 {
        store.add_clip(&format!("clip {i}"));
    }
    let clips = store.clips();
    assert_eq!(clips.len(), 500);
    assert_eq!(clips[0].text, "clip 504");
    assert_eq!(clips[499].text, "clip 5");
    assert!(clips.windows(2).all(|w| w[0].id > w[1].id));
}

#[test]
fn duplicate_clip_moves_to_top_with_a_new_id() {
    let store = Store::in_memory();
    store.add_clip("a");
    store.add_clip("b");
    let old_id = store.clips()[1].id;
    store.add_clip("a");
    let clips = store.clips();
    let texts: Vec<&str> = clips.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, ["a", "b"]);
    assert!(clips[0].id > clips[1].id);
    assert_eq!(store.clip_text(old_id), None);
    assert_eq!(store.clip_text(clips[0].id).as_deref(), Some("a"));
}

#[test]
fn blank_clips_are_skipped_but_whitespace_is_kept() {
    let store = Store::in_memory();
    store.add_clip("");
    store.add_clip(" \n\t ");
    store.add_clip("  padded  ");
    let texts: Vec<String> = store.clips().into_iter().map(|c| c.text).collect();
    assert_eq!(texts, ["  padded  "]);
}
