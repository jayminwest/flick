//! The task module's tables, `task_list`, `task_time` and `task_state`, and the only SQL
//! that touches them.

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::core::track::Span;
use crate::core::store::Store;

/// Append only.
pub const MIGRATIONS: &[&str] = &["CREATE TABLE task_list (
        id INTEGER PRIMARY KEY, title TEXT NOT NULL, project TEXT,
        status TEXT NOT NULL CHECK(status IN ('todo','doing','done')),
        created INTEGER, updated INTEGER, UNIQUE(title, project));
    CREATE TABLE task_time (
        id INTEGER PRIMARY KEY, task INTEGER NOT NULL REFERENCES task_list(id),
        start INTEGER NOT NULL, end INTEGER NOT NULL);
    CREATE INDEX task_time_start ON task_time (start);
    CREATE TABLE task_state (key TEXT PRIMARY KEY, value TEXT);"];

/// `task_state` key of the running task's id.
const RUNNING: &str = "running";
/// `task_state` key of the open `task_time` row's id.
const OPEN: &str = "open";

/// Where a task stands. Running is separate: at most one `Doing` task has an open row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Todo,
    /// Started at least once and not done.
    Doing,
    Done,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Done => "done",
        }
    }

    fn parse(s: &str) -> Status {
        match s {
            "doing" => Status::Doing,
            "done" => Status::Done,
            _ => Status::Todo,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub project: Option<String>,
    pub status: Status,
}

/// Tasks, their time rows and state on the shared store. Time writes ignore SQL errors: a
/// lost row must never break the event loop.
pub trait TaskStore {
    /// Add a todo task; its id. `Err` when the same title and project exist.
    fn task_add(&self, title: &str, project: Option<&str>, now: i64) -> Result<i64, String>;
    fn task_get(&self, id: i64) -> Option<Task>;
    /// Every task (`all`) or the ones not done: doing first, then most recently updated.
    fn task_list(&self, all: bool) -> Vec<Task>;
    fn task_set_status(&self, id: i64, status: Status, now: i64);
    /// Insert a time row `at..at` for task `task`; its row id.
    fn time_open(&self, task: i64, at: i64) -> Option<i64>;
    /// Move row `id`'s end to `at`, never before its start.
    fn time_end(&self, id: i64, at: i64);
    fn time_delete(&self, id: i64);
    /// Row `id` as a span of its task.
    fn time_row(&self, id: i64) -> Option<Span<i64>>;
    /// Time rows that overlap `from..to`, oldest first.
    fn times(&self, from: i64, to: i64) -> Vec<Span<i64>>;
    fn running(&self) -> Option<i64>;
    fn set_running(&self, id: Option<i64>);
    fn open_row(&self) -> Option<i64>;
    fn set_open_row(&self, id: Option<i64>);
    /// End the row stored as open at `at` and clear it (on quit, from a second connection).
    fn close_open(&self, at: i64);
}

fn task(r: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        project: r.get(2)?,
        status: Status::parse(&r.get::<_, String>(3)?),
    })
}

fn span(r: &rusqlite::Row) -> rusqlite::Result<Span<i64>> {
    Ok(Span { subject: r.get(0)?, start: r.get(1)?, end: r.get(2)? })
}

impl TaskStore for Store {
    fn task_add(&self, title: &str, project: Option<&str>, now: i64) -> Result<i64, String> {
        let conn = self.conn();
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM task_list WHERE title = ?1 AND project IS ?2",
                params![title, project],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("task: {e}"))?;
        if let Some(id) = exists {
            return Err(format!("task: task {id} is already called \"{title}\""));
        }
        conn.execute(
            "INSERT INTO task_list (title, project, status, created, updated)
             VALUES (?1, ?2, 'todo', ?3, ?3)",
            params![title, project, now],
        )
        .map_err(|e| format!("task: {e}"))?;
        Ok(conn.last_insert_rowid())
    }

    fn task_get(&self, id: i64) -> Option<Task> {
        self.conn()
            .query_row("SELECT id, title, project, status FROM task_list WHERE id = ?1", [id], task)
            .ok()
    }

    fn task_list(&self, all: bool) -> Vec<Task> {
        let Ok(mut stmt) = self.conn().prepare(
            "SELECT id, title, project, status FROM task_list WHERE ?1 OR status != 'done'
             ORDER BY status = 'doing' DESC, status = 'todo' DESC, updated DESC, id DESC",
        ) else {
            return vec![];
        };
        stmt.query_map([all], task).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    fn task_set_status(&self, id: i64, status: Status, now: i64) {
        let _ = self.conn().execute(
            "UPDATE task_list SET status = ?2, updated = ?3 WHERE id = ?1",
            params![id, status.as_str(), now],
        );
    }

    fn time_open(&self, task: i64, at: i64) -> Option<i64> {
        let conn = self.conn();
        conn.execute("INSERT INTO task_time (task, start, end) VALUES (?1, ?2, ?2)", [task, at])
            .ok()?;
        Some(conn.last_insert_rowid())
    }

    fn time_end(&self, id: i64, at: i64) {
        let _ =
            self.conn().execute("UPDATE task_time SET end = max(start, ?2) WHERE id = ?1", [id, at]);
    }

    fn time_delete(&self, id: i64) {
        let _ = self.conn().execute("DELETE FROM task_time WHERE id = ?1", [id]);
    }

    fn time_row(&self, id: i64) -> Option<Span<i64>> {
        self.conn()
            .query_row("SELECT task, start, end FROM task_time WHERE id = ?1", [id], span)
            .ok()
    }

    fn times(&self, from: i64, to: i64) -> Vec<Span<i64>> {
        let Ok(mut stmt) = self.conn().prepare(
            "SELECT task, start, end FROM task_time WHERE end > ?1 AND start < ?2 ORDER BY start, id",
        ) else {
            return vec![];
        };
        stmt.query_map([from, to], span).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    fn running(&self) -> Option<i64> {
        state(self, RUNNING)?.parse().ok()
    }

    fn set_running(&self, id: Option<i64>) {
        set_state(self, RUNNING, id.map(|id| id.to_string()).as_deref());
    }

    fn open_row(&self) -> Option<i64> {
        state(self, OPEN)?.parse().ok()
    }

    fn set_open_row(&self, id: Option<i64>) {
        set_state(self, OPEN, id.map(|id| id.to_string()).as_deref());
    }

    fn close_open(&self, at: i64) {
        if let Some(id) = self.open_row() {
            self.time_end(id, at);
        }
        set_state(self, OPEN, None);
    }
}

fn state(store: &Store, key: &str) -> Option<String> {
    store
        .conn()
        .query_row("SELECT value FROM task_state WHERE key = ?1", [key], |r| r.get(0))
        .ok()
        .flatten()
}

fn set_state(store: &Store, key: &str, value: Option<&str>) {
    let conn = store.conn();
    let _ = match value {
        Some(v) => conn.execute(
            "INSERT INTO task_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            [key, v],
        ),
        None => conn.execute("DELETE FROM task_state WHERE key = ?1", [key]),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let s = Store::in_memory();
        s.migrate("task", MIGRATIONS).unwrap();
        s
    }

    #[test]
    fn tasks_add_list_and_change_status() {
        let s = store();
        let plan = s.task_add("Write plan", Some("flick"), 10).unwrap();
        let other = s.task_add("Write plan", None, 20).unwrap();
        assert!(s.task_add("Write plan", Some("flick"), 30).unwrap_err().contains("already"));
        assert!(s.task_add("Write plan", None, 30).is_err());
        let mail = s.task_add("Mail", None, 40).unwrap();
        s.task_set_status(plan, Status::Doing, 50);
        s.task_set_status(mail, Status::Done, 60);
        let ids = |all| s.task_list(all).iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids(false), [plan, other]);
        assert_eq!(ids(true), [plan, other, mail]);
        let t = s.task_get(plan).unwrap();
        assert_eq!((t.title.as_str(), t.project.as_deref(), t.status), ("Write plan", Some("flick"), Status::Doing));
        assert_eq!(s.task_get(mail).unwrap().status, Status::Done);
        assert_eq!(s.task_get(99), None);
        assert_eq!(serde_json::to_string(&Status::Todo).unwrap(), r#""todo""#);
    }

    #[test]
    fn time_rows_round_trip_and_overlap() {
        let s = store();
        let t = s.task_add("a", None, 0).unwrap();
        let id = s.time_open(t, 100).unwrap();
        s.time_end(id, 160);
        let other = s.time_open(t, 200).unwrap();
        s.time_end(other, 260);
        let rows = s.times(150, 210);
        assert_eq!(rows.iter().map(|r| (r.start, r.end, r.subject)).collect::<Vec<_>>(), [(100, 160, t), (200, 260, t)]);
        assert!(s.times(260, 300).is_empty() && s.times(0, 100).is_empty());
        s.time_end(id, 50); // never before its start
        assert_eq!(s.time_row(id).map(|r| r.end), Some(100));
        s.time_delete(id);
        assert_eq!((s.time_row(id), s.times(0, 1000).len()), (None, 1));
    }

    #[test]
    fn state_is_empty_until_set_and_quit_closes_the_open_row() {
        let s = store();
        assert_eq!((s.running(), s.open_row()), (None, None));
        s.set_running(Some(3));
        assert_eq!(s.running(), Some(3));
        s.set_running(None);
        assert_eq!(s.running(), None);
        let t = s.task_add("a", None, 0).unwrap();
        let id = s.time_open(t, 10).unwrap();
        s.set_open_row(Some(id));
        assert_eq!(s.open_row(), Some(id));
        s.close_open(25);
        s.close_open(40); // nothing open: no change
        assert_eq!((s.times(0, 99)[0].end, s.open_row()), (25, None));
        // Without the tables everything reads as empty and writes do nothing.
        let bare = Store::in_memory();
        bare.set_running(Some(1));
        assert!(bare.running().is_none() && bare.times(0, 9).is_empty() && bare.task_list(true).is_empty());
        assert!(bare.time_open(1, 1).is_none() && bare.task_add("x", None, 0).is_err());
    }
}
