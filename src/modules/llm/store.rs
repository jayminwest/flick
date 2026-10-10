//! The normal chat's history (flick-6a0d): tables `llm_threads` and `llm_messages`, and the
//! only SQL that touches them. Private chats never reach this file: `PrivateSession` has no
//! store handle.

use rusqlite::{Row, params};

use crate::core::store::Store;

/// Append only.
pub const MIGRATIONS: &[&str] = &[
    // A thread: the server and model of its last request, its title (the first prompt,
    // clipped) and when it was started and last written. A message: its place in the thread
    // (`seq` from 0), `role` 'user' or 'assistant', the model that wrote or got it, its text,
    // and `state` NULL when done, 'stopped' (⌘. or a timeout kept what had arrived) or
    // 'failed' (`error` says why).
    "CREATE TABLE llm_threads (id TEXT PRIMARY KEY, server TEXT NOT NULL, model TEXT NOT NULL, title TEXT NOT NULL, created INTEGER NOT NULL, updated INTEGER NOT NULL);
     CREATE TABLE llm_messages (thread TEXT NOT NULL, seq INTEGER NOT NULL, ts INTEGER NOT NULL, role TEXT NOT NULL, model TEXT NOT NULL, body TEXT NOT NULL, state TEXT, error TEXT, PRIMARY KEY (thread, seq));",
];

/// Who wrote a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    User,
    Assistant,
}

/// How a message ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    Done,
    /// Stopped by hand: the text is what arrived.
    Stopped,
    /// The request failed; the text is what arrived before.
    Failed(String),
}

/// One message of a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Msg {
    pub who: Who,
    /// The model it was sent to or came from.
    pub model: String,
    pub body: String,
    pub end: End,
    /// Unix seconds.
    pub ts: i64,
}

/// A stored thread, for the thread list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub server: String,
    pub model: String,
    pub title: String,
    /// Unix seconds of its last write.
    pub updated: i64,
    pub messages: i64,
}

impl Who {
    fn column(self) -> &'static str {
        match self {
            Who::User => "user",
            Who::Assistant => "assistant",
        }
    }
}

impl End {
    fn columns(&self) -> (Option<&'static str>, Option<&str>) {
        match self {
            End::Done => (None, None),
            End::Stopped => (Some("stopped"), None),
            End::Failed(e) => (Some("failed"), Some(e)),
        }
    }

    /// Unknown values read as done.
    fn read(state: Option<&str>, error: Option<String>) -> End {
        match state {
            Some("stopped") => End::Stopped,
            Some("failed") => End::Failed(error.unwrap_or_default()),
            _ => End::Done,
        }
    }
}

fn msg(r: &Row) -> rusqlite::Result<Msg> {
    let who = if r.get::<_, String>(0)? == "assistant" { Who::Assistant } else { Who::User };
    Ok(Msg { who, model: r.get(1)?, body: r.get(2)?, end: End::read(r.get::<_, Option<String>>(3)?.as_deref(), r.get(4)?), ts: r.get(5)? })
}

/// What `llm_save` writes: message `seq` of thread `id` (created with `title` if new).
pub struct Save<'a> {
    pub id: &'a str,
    pub server: &'a str,
    pub title: &'a str,
    pub seq: usize,
    pub msg: &'a Msg,
    /// Threads kept (`max_threads`), this one among them.
    pub keep: usize,
}

/// The normal chat's history on the shared store.
pub trait Chats {
    /// Write one message (replacing the one at its `seq`), move the thread to the top, then
    /// drop the oldest threads past `keep` (never this one) with their messages.
    fn llm_save(&self, s: &Save) -> Result<(), String>;
    /// At most `limit` threads, the last written first.
    fn llm_threads(&self, limit: usize) -> Vec<Thread>;
    /// Thread `id` and its messages in order.
    fn llm_thread(&self, id: &str) -> Option<(Thread, Vec<Msg>)>;
}

fn n(k: usize) -> i64 {
    i64::try_from(k).unwrap_or(i64::MAX)
}

const THREAD: &str = "SELECT t.id, t.server, t.model, t.title, t.updated, (SELECT COUNT(*) FROM llm_messages m WHERE m.thread = t.id) FROM llm_threads t";

fn thread(r: &Row) -> rusqlite::Result<Thread> {
    Ok(Thread { id: r.get(0)?, server: r.get(1)?, model: r.get(2)?, title: r.get(3)?, updated: r.get(4)?, messages: r.get(5)? })
}

impl Chats for Store {
    fn llm_save(&self, s: &Save) -> Result<(), String> {
        let m = s.msg;
        let (state, error) = m.end.columns();
        let fail = |e: rusqlite::Error| format!("llm: can't save the chat: {e}");
        let tx = self.conn().unchecked_transaction().map_err(fail)?;
        tx.execute(
            "INSERT INTO llm_threads (id, server, model, title, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?5) ON CONFLICT(id) DO UPDATE SET server = excluded.server, model = excluded.model, updated = excluded.updated",
            params![s.id, s.server, m.model, s.title, m.ts],
        )
        .map_err(fail)?;
        tx.execute(
            "INSERT OR REPLACE INTO llm_messages (thread, seq, ts, role, model, body, state, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![s.id, n(s.seq), m.ts, m.who.column(), m.model, m.body, state, error],
        )
        .map_err(fail)?;
        let old = "SELECT id FROM llm_threads WHERE id <> ?2 ORDER BY updated DESC, rowid DESC LIMIT -1 OFFSET ?1";
        let keep = n(s.keep.saturating_sub(1));
        tx.execute(&format!("DELETE FROM llm_messages WHERE thread IN ({old})"), params![keep, s.id]).map_err(fail)?;
        tx.execute(&format!("DELETE FROM llm_threads WHERE id IN ({old})"), params![keep, s.id]).map_err(fail)?;
        tx.commit().map_err(fail)
    }

    fn llm_threads(&self, limit: usize) -> Vec<Thread> {
        let sql = format!("{THREAD} ORDER BY t.updated DESC, t.rowid DESC LIMIT ?1");
        let Ok(mut stmt) = self.conn().prepare(&sql) else { return vec![] };
        stmt.query_map([n(limit)], thread).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    fn llm_thread(&self, id: &str) -> Option<(Thread, Vec<Msg>)> {
        let conn = self.conn();
        let t = conn.query_row(&format!("{THREAD} WHERE t.id = ?1"), [id], thread).ok()?;
        let sql = "SELECT role, model, body, state, error, ts FROM llm_messages WHERE thread = ?1 ORDER BY seq";
        let mut stmt = conn.prepare(sql).ok()?;
        let list = stmt.query_map([id], msg).map(|rows| rows.flatten().collect()).unwrap_or_default();
        Some((t, list))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let s = Store::in_memory();
        s.migrate("llm", MIGRATIONS).unwrap();
        s
    }

    fn m(who: Who, body: &str, end: End, ts: i64) -> Msg {
        Msg { who, model: "qwen".into(), body: body.into(), end, ts }
    }

    fn save(s: &Store, id: &str, seq: usize, msg: &Msg, keep: usize) {
        s.llm_save(&Save { id, server: "mlx", title: "Hello", seq, msg, keep }).unwrap();
    }

    fn ids(s: &Store) -> Vec<String> {
        s.llm_threads(100).into_iter().map(|t| t.id).collect()
    }

    #[test]
    fn a_thread_round_trips_in_order() {
        let s = store();
        let q = m(Who::User, "hi", End::Done, 10);
        let a = m(Who::Assistant, "yo", End::Failed("boom".into()), 11);
        save(&s, "l1", 0, &q, 5);
        save(&s, "l1", 1, &a, 5);
        let stopped = m(Who::Assistant, "par", End::Stopped, 12);
        save(&s, "l1", 2, &stopped, 5);
        let (t, list) = s.llm_thread("l1").unwrap();
        let want = Thread { id: "l1".into(), server: "mlx".into(), model: "qwen".into(), title: "Hello".into(), updated: 12, messages: 3 };
        assert_eq!(t, want);
        assert_eq!(list, [q, a, stopped]);
        assert!(s.llm_thread("nope").is_none());
        // Unknown states written by a later version read as done.
        s.conn().execute_batch("UPDATE llm_messages SET state = 'x' WHERE seq = 2").unwrap();
        assert_eq!(s.llm_thread("l1").unwrap().1[2].end, End::Done);
    }

    #[test]
    fn a_rewritten_seq_replaces_and_the_thread_moves_up() {
        let s = store();
        save(&s, "a", 0, &m(Who::User, "1", End::Done, 1), 5);
        save(&s, "b", 0, &m(Who::User, "2", End::Done, 2), 5);
        assert_eq!(ids(&s), ["b", "a"]);
        save(&s, "a", 0, &m(Who::User, "1b", End::Done, 3), 5);
        assert_eq!(ids(&s), ["a", "b"]);
        let (t, list) = s.llm_thread("a").unwrap();
        assert_eq!((t.messages, list[0].body.as_str(), t.title.as_str()), (1, "1b", "Hello"));
    }

    #[test]
    fn the_oldest_threads_past_the_cap_go_with_their_messages() {
        let s = store();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            save(&s, id, 0, &m(Who::User, id, End::Done, i as i64), 2);
        }
        assert_eq!(ids(&s), ["c", "b"]);
        let left: i64 = s.conn().query_row("SELECT COUNT(*) FROM llm_messages", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 2);
        // The thread written stays even when its time is the oldest.
        save(&s, "z", 0, &m(Who::User, "z", End::Done, -5), 1);
        assert_eq!(ids(&s), ["z"]);
        assert_eq!(s.llm_threads(0), []);
    }

    #[test]
    fn a_broken_table_is_an_error_not_a_panic() {
        let s = Store::in_memory();
        let err = s.llm_save(&Save { id: "a", server: "x", title: "t", seq: 0, msg: &m(Who::User, "q", End::Done, 1), keep: 1 });
        assert!(err.unwrap_err().starts_with("llm: can't save the chat: "));
        assert_eq!(s.llm_threads(5), []);
        assert!(s.llm_thread("a").is_none());
    }
}
