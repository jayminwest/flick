//! The `messages` table: history, trimming, threads, replacement in place.

use super::*;

fn store() -> Store {
    let s = Store::in_memory();
    s.migrate("message", MIGRATIONS).unwrap();
    s
}

fn keep(history: usize) -> Keep {
    Keep { history, per_thread: 3, threads: 2 }
}

fn msg(id: &str, ts: i64) -> Message {
    Message { id: id.into(), ts, body: format!("body {id}"), ..Message::default() }
}

#[test]
fn history_is_newest_first_and_trimmed() {
    let s = store();
    for (i, id) in ["a", "b", "c"].iter().enumerate() {
        s.put_message(&msg(id, 100 + i as i64), keep(2)).unwrap();
    }
    let ids: Vec<_> = s.messages(10).into_iter().map(|m| m.id).collect();
    assert_eq!(ids, ["c", "b"]);
    assert_eq!(s.messages(1).len(), 1);
    assert!(s.message("a").is_none());
    // Same second: the later insert is newer.
    s.put_message(&msg("d", 102), keep(5)).unwrap();
    assert_eq!(s.messages(1)[0].id, "d");
}

#[test]
fn only_pending_messages_are_taken() {
    let s = store();
    s.put_message(&Message { pending: true, ..msg("p", 1) }, keep(5)).unwrap();
    s.put_message(&msg("q", 2), keep(5)).unwrap();
    assert!(s.take_pending("q").is_none());
    assert_eq!(s.take_pending("p").map(|m| m.body), Some("body p".into()));
    assert!(s.message("p").is_none());
    assert!(s.take_pending("p").is_none());
    assert_eq!(s.message("q").map(|m| m.pending), Some(false));
}

#[test]
fn cards_keep_their_json_and_origin() {
    let s = store();
    let card = Message { card: Some(r#"{"id":"c","title":"T"}"#.into()), remote: true, ..msg("c", 2) };
    s.put_message(&msg("t", 1), keep(5)).unwrap();
    s.put_message(&card, keep(5)).unwrap();
    assert_eq!(s.message("c"), Some(card.clone()));
    assert_eq!(s.cards(10), [card]);
    assert_eq!(s.messages(10).len(), 2);
    // `ls --json`: the card as an object, `remote` only when set.
    let json = serde_json::to_string(&s.messages(10)).unwrap();
    assert!(json.contains(r#""card":{"id":"c","title":"T"},"remote":true}"#), "{json}");
    assert!(json.ends_with(r#""pending":false}]"#), "{json}");
}

#[test]
fn migration_2_keeps_old_rows_as_local_text_messages() {
    let s = Store::in_memory();
    s.migrate("message", &MIGRATIONS[..1]).unwrap();
    s.conn().execute_batch("INSERT INTO messages (id, ts, body) VALUES ('old', 1, 'hi')").unwrap();
    s.migrate("message", MIGRATIONS).unwrap();
    let old = s.message("old").unwrap();
    assert_eq!((old.card, old.remote, old.body.as_str()), (None, false, "hi"));
}
fn in_thread(id: &str, ts: i64, thread: &str) -> Message {
    Message { thread: Some(thread.into()), ..msg(id, ts) }
}

fn ids(list: &[Message]) -> Vec<&str> {
    list.iter().map(|m| m.id.as_str()).collect()
}

#[test]
fn migration_3_adds_chat_columns_and_threads_old_cards() {
    let s = Store::in_memory();
    s.migrate("message", &MIGRATIONS[..2]).unwrap();
    s.conn()
        .execute_batch(
            r#"INSERT INTO messages (id, ts, body) VALUES ('old', 1, 'hi');
               INSERT INTO messages (id, ts, body, card) VALUES ('c', 2, 'C', '{"id":"c","title":"C","thread":"ops"}');
               INSERT INTO messages (id, ts, body, card) VALUES ('bad', 3, 'B', 'not json');"#,
        )
        .unwrap();
    s.migrate("message", MIGRATIONS).unwrap();
    let old = s.message("old").unwrap();
    assert_eq!((old.thread, old.role, old.state), (None, Role::Peer, Progress::Done));
    assert_eq!(s.message("c").unwrap().thread.as_deref(), Some("ops"));
    assert_eq!(s.message("bad").unwrap().thread, None);
    assert_eq!(ids(&s.thread("ops", 10)), ["c"]);
}

#[test]
fn chat_fields_round_trip_and_serialize_only_when_set() {
    let s = store();
    let me = Message { role: Role::Me, state: Progress::Failed, ..in_thread("q", 1, "t1") };
    let partial = Message { state: Progress::Partial, ..in_thread("a", 2, "t1") };
    s.put_message(&me, keep(5)).unwrap();
    s.put_message(&partial, keep(5)).unwrap();
    s.put_message(&msg("plain", 3), keep(5)).unwrap();
    assert_eq!(s.message("q"), Some(me));
    assert_eq!(s.message("a"), Some(partial));
    // Unknown values written by a later version read as the defaults.
    s.conn().execute_batch("UPDATE messages SET role = 'bot', state = 'x' WHERE id = 'q'").unwrap();
    let q = s.message("q").unwrap();
    assert_eq!((q.role, q.state), (Role::Peer, Progress::Done));
    let json = serde_json::to_string(&s.thread("t1", 10)).unwrap();
    assert!(json.ends_with(r#""pending":false,"thread":"t1","state":"partial"}]"#), "{json}");
    let json = serde_json::to_string(&s.message("plain")).unwrap();
    assert!(json.ends_with(r#""pending":false}"#), "{json}");
    s.put_message(&Message { role: Role::Me, ..in_thread("q", 1, "t1") }, keep(5)).unwrap();
    let json = serde_json::to_string(&s.message("q")).unwrap();
    assert!(json.ends_with(r#""thread":"t1","role":"me"}"#), "{json}");
}

#[test]
fn a_threaded_or_partial_repost_keeps_its_time_and_place() {
    let s = store();
    s.put_message(&in_thread("q", 10, "t"), keep(5)).unwrap();
    s.put_message(&Message { state: Progress::Partial, ..in_thread("a", 11, "t") }, keep(5)).unwrap();
    s.put_message(&in_thread("card", 11, "t"), keep(5)).unwrap();
    // Streaming: `a` again at a later time, then its final post without a thread.
    s.put_message(&Message { state: Progress::Partial, body: "more".into(), ..in_thread("a", 20, "t") }, keep(5))
        .unwrap();
    s.put_message(&Message { body: "all".into(), ..msg("a", 30) }, keep(5)).unwrap();
    let a = s.message("a").unwrap();
    assert_eq!((a.ts, a.body.as_str(), a.thread.as_deref(), a.state), (11, "all", Some("t"), Progress::Done));
    assert_eq!(ids(&s.thread("t", 10)), ["q", "a", "card"], "same second: insert order");
    // An unthreaded partial stream keeps its time too, until it is done.
    s.put_message(&msg("x", 40), keep(5)).unwrap();
    s.put_message(&Message { state: Progress::Partial, ..msg("u", 41) }, keep(5)).unwrap();
    s.put_message(&msg("y", 42), keep(5)).unwrap();
    s.put_message(&msg("u", 43), keep(5)).unwrap();
    assert_eq!(ids(&s.messages(3)), ["y", "u", "x"]);
    assert_eq!(s.message("u").unwrap().ts, 41);
    // A done unthreaded message posted again is new, as before chat.
    s.put_message(&msg("u", 44), keep(5)).unwrap();
    assert_eq!(ids(&s.messages(2)), ["u", "y"]);
    // A post may move a message into another thread.
    s.put_message(&in_thread("card", 50, "t2"), keep(5)).unwrap();
    assert_eq!(ids(&s.thread("t2", 10)), ["card"]);
    assert_eq!(s.message("card").unwrap().ts, 11);
}

#[test]
fn history_is_trimmed_per_scope() {
    let s = store();
    // per_thread 3, threads 2, history 2.
    for i in 0..5 {
        s.put_message(&in_thread(&format!("a{i}"), 100 + i, "ta"), keep(2)).unwrap();
    }
    assert_eq!(ids(&s.thread("ta", 10)), ["a2", "a3", "a4"]);
    assert_eq!(ids(&s.thread("ta", 2)), ["a3", "a4"]);
    for id in ["h1", "h2", "h3"] {
        s.put_message(&msg(id, 200), keep(2)).unwrap();
    }
    // Unthreaded posts trim only unthreaded history; chat never evicts it.
    assert_eq!(s.thread("ta", 10).len(), 3);
    s.put_message(&in_thread("b0", 300, "tb"), keep(2)).unwrap();
    s.put_message(&in_thread("c0", 301, "tc"), keep(2)).unwrap();
    let threads: Vec<_> = s.threads(10).into_iter().map(|t| t.id).collect();
    assert_eq!(threads, ["tc", "tb"], "ta had the oldest last message");
    assert!(s.message("a4").is_none());
    assert_eq!(ids(&s.messages(10)), ["c0", "b0", "h3", "h2"]);
    // The thread posted to stays even when its times are the oldest.
    s.put_message(&Message { state: Progress::Partial, ..in_thread("b1", 302, "tb") }, keep(2)).unwrap();
    s.put_message(&in_thread("old", 1, "tb"), keep(2)).unwrap();
    s.put_message(&in_thread("old", 1, "tb"), Keep { history: 2, per_thread: 9, threads: 1 }).unwrap();
    let threads: Vec<_> = s.threads(10).into_iter().map(|t| t.id).collect();
    assert_eq!(threads, ["tb"]);
}

#[test]
fn threads_list_newest_activity_first() {
    let s = store();
    assert_eq!(s.threads(5), []);
    assert_eq!(s.thread("none", 5), []);
    s.put_message(&Message { role: Role::Me, body: "first?".into(), ..in_thread("q1", 10, "t1") }, keep(5)).unwrap();
    s.put_message(&in_thread("r1", 12, "t1"), keep(5)).unwrap();
    s.put_message(&in_thread("q2", 11, "t2"), keep(5)).unwrap();
    s.put_message(&msg("loose", 13), keep(5)).unwrap();
    let t = s.threads(5);
    let first = Thread { id: "t1".into(), messages: 2, first_ts: 10, last_ts: 12, first_body: "first?".into() };
    assert_eq!(t[0], first);
    assert_eq!(t.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t1", "t2"]);
    assert_eq!(s.threads(1).len(), 1);
    let json = serde_json::to_string(&s.threads(1)).unwrap();
    assert_eq!(json, r#"[{"id":"t1","messages":2,"first_ts":10,"last_ts":12,"first_body":"first?"}]"#);
}

#[test]
fn a_failed_write_is_an_error_not_a_panic() {
    let s = Store::in_memory();
    let err = s.put_message(&msg("a", 1), keep(2)).unwrap_err();
    assert!(err.starts_with("message: can't save: "), "{err}");
    let s = store();
    s.put_message(&msg("a", 1), keep(1)).unwrap();
    s.conn().execute_batch("CREATE TRIGGER no_trim BEFORE DELETE ON messages BEGIN SELECT RAISE(FAIL, 'held'); END;").unwrap();
    assert_eq!(s.put_message(&msg("b", 2), keep(1)), Err("message: can't trim history: held".into()));
}

#[test]
fn restamp_moves_a_message_in_time() {
    let s = store();
    s.put_message(&in_thread("q", 10, "t"), keep(5)).unwrap();
    s.put_message(&in_thread("r", 20, "t"), keep(5)).unwrap();
    s.restamp("q", 30);
    assert_eq!(ids(&s.thread("t", 5)), ["r", "q"]);
}
