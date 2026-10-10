use super::*;

/// 2026-10-09 18:31:01 UTC.
const NOW: i64 = 1_791_570_661;

fn utc(_: i64) -> i32 {
    0
}

fn clock(now: i64) -> Clock {
    Clock { now, offset: utc }
}

fn me(id: &str, ts: i64, body: &str) -> Message {
    Message { id: id.into(), ts, body: body.into(), thread: Some("t1".into()), role: Role::Me, ..Message::default() }
}

fn peer(id: &str, ts: i64, body: &str) -> Message {
    Message { id: id.into(), ts, body: body.into(), thread: Some("t1".into()), ..Message::default() }
}

fn thread(id: &str) -> Thread {
    Thread { id: id.into(), messages: 1, first_ts: 0, last_ts: 0, first_body: String::new() }
}

fn bubble(row: &Row) -> (&str, Side, &str, &str, &str, State) {
    match row {
        Row::Bubble { key, side, header, time, md, state, .. } => (key, *side, header, time, md, *state),
        other => panic!("not a bubble: {other:?}"),
    }
}

#[test]
fn a_thread_reads_as_bubbles_under_day_dividers() {
    let list = [
        me("q1", NOW - 86_400, "yesterday?"),
        peer("a1", NOW - 86_400 + 60, "**yes**"),
        me("q2", NOW - 120, "and today?"),
        Message { title: Some("Agent".into()), ..peer("a2", NOW - 60, "today") },
    ];
    let rows = transcript(&list, "KOTA", clock(NOW));
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0], Row::Divider { text: "Yesterday".into() });
    assert_eq!(bubble(&rows[1]), ("q1", Side::Mine, "", "18:31", "yesterday?", State::Done));
    assert_eq!(bubble(&rows[2]), ("a1", Side::Theirs, "KOTA", "18:32", "**yes**", State::Done));
    assert_eq!(rows[3], Row::Divider { text: "Today".into() });
    assert_eq!(bubble(&rows[4]).0, "q2");
    assert_eq!(bubble(&rows[5]), ("a2", Side::Theirs, "Agent", "18:30", "today", State::Done));
    assert_eq!(transcript(&[], "KOTA", clock(NOW)), []);
}

#[test]
fn days_and_times_are_local() {
    fn pdt(_: i64) -> i32 {
        -25_200
    }
    // 18:31 UTC is 11:31 PDT; 06:31 UTC is still yesterday in PDT.
    let list = [me("q1", NOW - 12 * 3600, "late"), me("q2", NOW, "now")];
    let rows = transcript(&list, "KOTA", Clock { now: NOW, offset: pdt });
    assert_eq!(rows[0], Row::Divider { text: "Yesterday".into() });
    assert_eq!(bubble(&rows[1]).3, "23:31");
    assert_eq!(rows[2], Row::Divider { text: "Today".into() });
    assert_eq!(bubble(&rows[3]).3, "11:31");
}

#[test]
fn peer_posts_show_their_progress() {
    let list = [
        Message { pending: true, ..peer("p", NOW, "sent to KOTA") },
        Message { state: Progress::Partial, ..peer("s", NOW, "so far") },
        Message { state: Progress::Failed, ..peer("f", NOW, "broke") },
        Message { pending: true, state: Progress::Partial, ..peer("pp", NOW, "both") },
    ];
    let rows = transcript(&list, "KOTA", clock(NOW));
    let states: Vec<State> = rows[1..].iter().map(|r| bubble(r).5).collect();
    assert_eq!(states, [State::Pending, State::Streaming, State::Failed, State::Pending]);
}

#[test]
fn a_question_that_did_not_go_out_says_so_and_can_be_retried() {
    let list = [me("q1", NOW - 5, "first"), peer("a1", NOW - 4, "ok"), Message { state: Progress::Failed, ..me("q2", NOW, "second") }];
    let rows = transcript(&list, "KOTA", clock(NOW));
    assert_eq!(rows.len(), 4, "no thinking bubble for a failed ask");
    assert_eq!(bubble(&rows[3]), ("q2", Side::Mine, NOT_SENT, "18:31", "second", State::Failed));
    assert_eq!(retry(&list).map(|m| m.id.as_str()), Some("q2"));
    assert_eq!(status(&list, NOW), Status::Error);
    // Only the newest question is retried, and only when it failed.
    let later = [list[2].clone(), me("q3", NOW, "third")];
    assert_eq!(retry(&later), None);
    assert_eq!(retry(&list[..2]), None);
    assert_eq!(retry(&[]), None);
}

#[test]
fn an_unanswered_question_shows_thinking_for_a_while() {
    let list = [me("q1", NOW - 10, "hm?")];
    let rows = transcript(&list, "KOTA", clock(NOW));
    assert_eq!(bubble(&rows[2]), ("thinking:q1", Side::Theirs, "KOTA", "", THINKING, State::Pending));
    assert_eq!(status(&list, NOW), Status::Busy);
    // Gone once anything from the peer follows it, or after THINKING_SECS.
    let answered = [list[0].clone(), Message { pending: true, ..peer("a", NOW, "…") }];
    assert_eq!(transcript(&answered, "KOTA", clock(NOW)).len(), 3);
    let late = clock(NOW - 10 + THINKING_SECS);
    assert_eq!(transcript(&list, "KOTA", late).len(), 2);
    assert_eq!(status(&list, late.now), Status::Idle);
}

#[test]
fn the_header_is_busy_while_a_reply_streams() {
    let streaming = [me("q", NOW - 9000, "?"), Message { state: Progress::Partial, ..peer("a", NOW, "so") }];
    assert_eq!(status(&streaming, NOW), Status::Busy);
    let pending = [Message { pending: true, ..peer("a", NOW, "…") }];
    assert_eq!(status(&pending, NOW), Status::Busy);
    let done = [me("q", NOW - 5, "?"), peer("a", NOW, "!")];
    assert_eq!(status(&done, NOW), Status::Idle);
    assert_eq!(status(&[], NOW), Status::Idle);
}

#[test]
fn cards_are_card_rows_and_bad_ones_fall_back_to_text() {
    let good = Message { card: Some(r#"{"id":"c","title":"Pick one"}"#.into()), ..peer("c", NOW, "Pick one") };
    let bad = Message { card: Some("not json".into()), ..peer("b", NOW, "plain body") };
    let rows = transcript(&[good.clone(), bad], "KOTA", clock(NOW));
    match &rows[1] {
        Row::Card { key, version: v, card } => {
            assert_eq!((key.as_str(), card.title.as_str()), ("c", "Pick one"));
            assert_eq!(*v, version(&good));
        }
        other => panic!("not a card: {other:?}"),
    }
    assert_eq!(bubble(&rows[2]).4, "plain body");
}

#[test]
fn versions_change_with_what_a_row_shows() {
    let m = peer("a", NOW, "one");
    let v = version(&m);
    assert_eq!(v, version(&m.clone()), "stable");
    let changed = [
        Message { body: "two".into(), ..m.clone() },
        Message { title: Some("T".into()), ..m.clone() },
        Message { card: Some("{}".into()), ..m.clone() },
        Message { ts: NOW + 1, ..m.clone() },
        Message { pending: true, ..m.clone() },
        Message { role: Role::Me, ..m.clone() },
        Message { state: Progress::Partial, ..m.clone() },
        Message { state: Progress::Failed, ..m.clone() },
    ];
    for c in &changed {
        assert_ne!(version(c), v, "{c:?}");
    }
    assert_ne!(version(&changed[6]), version(&changed[7]));
    assert_ne!(fold(v, &"pending"), v);
    assert_eq!(fold(v, &"pending"), fold(v, &"pending"));
    assert_ne!(fold(v, &"pending"), fold(v, &"idle"));
}

#[test]
fn titles_are_the_first_question_on_one_line() {
    assert_eq!(title("What's on\nmy **calendar**?"), "What's on my calendar?");
    assert_eq!(title("  "), UNTITLED);
    let long = title(&"word ".repeat(40));
    assert_eq!(long.chars().count(), TITLE_MAX);
    assert!(long.ends_with('…'));
}

#[test]
fn threads_step_older_and_newer() {
    let list = [thread("new"), thread("mid"), thread("old")];
    assert_eq!(neighbor(&list, Some("new"), Step::Older), Some("mid"));
    assert_eq!(neighbor(&list, Some("mid"), Step::Older), Some("old"));
    assert_eq!(neighbor(&list, Some("old"), Step::Older), None);
    assert_eq!(neighbor(&list, Some("mid"), Step::Newer), Some("new"));
    assert_eq!(neighbor(&list, Some("new"), Step::Newer), None);
    // A thread not stored yet sits before the newest one.
    assert_eq!(neighbor(&list, Some("fresh"), Step::Older), Some("new"));
    assert_eq!(neighbor(&list, None, Step::Older), Some("new"));
    assert_eq!(neighbor(&list, None, Step::Newer), None);
    assert_eq!(neighbor(&[], None, Step::Older), None);
}

#[test]
fn the_open_thread_never_alerts() {
    let quiet = Alert { hud: false, sound: false };
    let reply = peer("a", NOW, "hi");
    assert_eq!(alert(&reply, true, Some("t1")), quiet);
    assert_eq!(alert(&me("q", NOW, "?"), true, None), quiet, "the user's own question");
    assert_eq!(alert(&me("q", NOW, "?"), true, Some("t2")), quiet);
}

#[test]
fn other_threads_alert_on_the_first_and_final_post() {
    let partial = Message { state: Progress::Partial, ..peer("a", NOW, "so") };
    let pending = Message { pending: true, ..peer("a", NOW, "…") };
    let done = peer("a", NOW, "all");
    for open in [None, Some("t2")] {
        assert_eq!(alert(&partial, true, open), Alert { hud: true, sound: false });
        assert_eq!(alert(&partial, false, open), Alert { hud: false, sound: false });
        assert_eq!(alert(&pending, true, open), Alert { hud: true, sound: false });
        assert_eq!(alert(&done, false, open), Alert { hud: true, sound: true });
        assert_eq!(alert(&done, true, open), Alert { hud: true, sound: true });
    }
}

#[test]
fn unthreaded_posts_keep_the_hud_behaviour() {
    let loose = |m: Message| Message { thread: None, ..m };
    let partial = loose(Message { state: Progress::Partial, ..peer("a", NOW, "so") });
    assert_eq!(alert(&partial, false, Some("t1")), Alert { hud: true, sound: false });
    assert_eq!(alert(&loose(Message { pending: true, ..peer("p", NOW, "…") }), true, None), Alert { hud: true, sound: false });
    assert_eq!(alert(&loose(peer("d", NOW, "!")), false, Some("t1")), Alert { hud: true, sound: true });
}

#[test]
fn thread_ids_are_short_valid_and_distinct() {
    assert_eq!(base36(0), "0");
    assert_eq!(base36(36 * 36 + 35), "10z");
    assert_eq!(thread_id(36, 37), "t101");
    let (a, b) = (new_thread_id(), new_thread_id());
    assert_ne!(a, b);
    assert!(a.starts_with('t') && crate::core::card::valid_id(&a), "{a}");
}
