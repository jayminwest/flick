//! A thread's corner cards close when the window shows it; ⌘R restarts "Thinking…"
//! (flick-1947).

use super::*;
use crate::modules::message::tests::TS;

#[test]
fn showing_a_thread_closes_its_corner_cards_without_storing_a_dismissal() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "a", "first"]).unwrap();
    let card = r#"{"id":"c1","title":"Ship?","thread":"t1","actions":[{"id":"go","label":"Ship"}]}"#;
    f.run(&mut m, false, &["card", "post", card]).unwrap();
    f.run(&mut m, false, &["post", "--thread", "t2", "--id", "b", "other"]).unwrap();
    f.run(&mut m, false, &["post", "--id", "loose", "unthreaded"]).unwrap();
    assert_eq!(only("show", &take_log()).len(), 3, "the window is hidden: corner cards");
    f.cx("", false, |cx| m.summon(Some("t1".into()), cx));
    assert_eq!(only("dismiss", &take_log()), ["a", "c1"], "only t1's, and not the user's own");
    assert_eq!((f.dismissal("a"), f.dismissal("c1")), (None, None));
    assert!(f.store.message("loose").unwrap().unread, "an unthreaded post stays unread");
    // ⌘] onto t2 closes its card too; ⌘N (an empty thread) closes nothing.
    queue(Note::Key(Command::Newer));
    event(&mut f, &mut m);
    assert_eq!(only("dismiss", &take_log()), ["b"]);
    queue(Note::Key(Command::New));
    event(&mut f, &mut m);
    assert!(only("dismiss", &take_log()).is_empty());
}

#[test]
fn a_retry_restarts_thinking_and_moves_the_question_to_its_time() {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nkota_host = \"down@host\""));
    f.cx("", false, |cx| m.summon(Some("t1".into()), cx));
    queue(Note::Submit("ping".into()));
    settle(&mut f, &mut m);
    // KOTA said something in the thread meanwhile; the question failed an hour ago.
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "k1", "hello"]).unwrap();
    f.store.restamp("m1", TS - 3600);
    take_log();
    m.settings.kota_host = "ok@host".into();
    m.env.now = || TS + 60;
    queue(Note::Key(Command::Retry));
    settle(&mut f, &mut m);
    assert_eq!(f.store.message("m1").map(|q| (q.ts, q.state)), Some((TS + 60, Progress::Done)));
    let log = take_log();
    assert_eq!(only("wake", &log), [(model::THINKING_SECS + 1).to_string()]);
    let rows = only("chat rows", &log).pop().unwrap();
    assert_eq!(rows, "-- Today / k1 Done|Theirs|Messages|hello / m1 Done|Mine||ping / thinking:m1 Pending|Theirs|Messages|Thinking…");
    assert!(only("chat header", &log).last().unwrap().ends_with("|busy"));
}
