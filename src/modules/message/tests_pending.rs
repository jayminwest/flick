//! The count of cards waiting on the user (`pending.rs`), as `Event::CardsPending` counts
//! the fake `pending` hook records.

use super::dispatch::Note;
use super::tests::{Fixture, TS, inbox, queue, take_log, take_pending};
use super::tests_press::{ASK, posted, press, settle};
use super::*;

fn started(f: &mut Fixture, m: &mut Inbox) {
    f.cx("", false, |cx| assert!(!m.on_event(Event::Started, cx)));
}

#[test]
fn started_sends_the_first_count_and_changes_only_after() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    started(&mut f, &mut m);
    assert_eq!(take_pending(), [0]);
    // Other events are ignored; an unchanged count is not sent again.
    f.cx("", false, |cx| m.on_event(Event::Wake, cx));
    started(&mut f, &mut m);
    f.run(&mut m, false, &["post", "hello"]).unwrap();
    assert_eq!(take_pending(), [0; 0]);
    // An open card with an enabled action waits on the user.
    f.run(&mut m, false, &["card", "post", ASK]).unwrap();
    assert_eq!(take_pending(), [1]);
    // Done, action-less and only-disabled cards do not.
    for other in [
        r#"{"id":"c2","title":"Shipped","state":"done","actions":[{"id":"ok","label":"OK"}]}"#,
        r#"{"id":"c3","title":"FYI"}"#,
        r#"{"id":"c4","title":"Off","actions":[{"id":"x","label":"X","do":{"open_url":"ftp://x"}}]}"#,
    ] {
        f.run(&mut m, false, &["card", "post", other]).unwrap();
    }
    assert_eq!(take_pending(), [0; 0]);
    f.cx("", false, |cx| assert_eq!(m.waiting(cx), 1));
    // A failed verb still re-counts (nothing changed, so nothing is sent).
    assert!(f.run(&mut m, false, &["card", "dismiss", "nope"]).is_err());
    assert_eq!(take_pending(), [0; 0]);
    take_log();
}

#[test]
fn a_card_waiting_on_kota_does_not_count() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]\npending_timeout_secs = 30");
    assert_eq!(take_pending(), [1]);
    press("go");
    settle(&mut f, &mut m);
    assert_eq!(take_pending(), [0]);
    // KOTA's silence turns into an error line: the user is up again.
    m.env.now = || TS + 30;
    settle(&mut f, &mut m);
    assert_eq!(take_pending(), [1]);
    // A press that KOTA answers with a re-post: busy, then waiting again.
    press("go");
    settle(&mut f, &mut m);
    f.run(&mut m, false, &["card", "post", ASK]).unwrap();
    assert_eq!(take_pending(), [0, 1]);
    // The shell confirm step still waits on the user.
    press("sh");
    settle(&mut f, &mut m);
    assert_eq!(take_pending(), [0; 0]);
    take_log();
}

#[test]
fn a_timed_out_card_still_waits_but_a_dismissed_one_does_not() {
    let (mut f, mut m) = posted("");
    take_pending();
    queue(Note::Expired("c1".into()));
    settle(&mut f, &mut m);
    // Gone from the corner like a dismissal (a done update stays silent)...
    assert!(m.dismissed.contains("c1") && m.expired.contains("c1"));
    assert_eq!(take_pending(), [0; 0]);
    // ...until the user dismisses it.
    queue(Note::Dismissed("c1".into()));
    settle(&mut f, &mut m);
    assert!(!m.expired.contains("c1"));
    assert_eq!(take_pending(), [0]);
    // An open re-post brings it back.
    f.run(&mut m, false, &["card", "post", ASK]).unwrap();
    assert_eq!(take_pending(), [1]);
    assert_eq!(f.run(&mut m, false, &["card", "dismiss", "c1"]), Ok("Dismissed c1".into()));
    assert_eq!(take_pending(), [0]);
    queue(Note::Expired("c1".into()));
    settle(&mut f, &mut m);
    f.run(&mut m, false, &["card", "post", ASK]).unwrap();
    assert!(!m.expired.contains("c1"));
    take_log();
}

#[test]
fn dismissals_are_forgotten_on_restart() {
    let (mut f, mut m) = posted("");
    f.run(&mut m, false, &["card", "dismiss", "--all"]).unwrap();
    assert_eq!(take_pending(), [1, 0]);
    // A new process over the same store counts the open card again.
    let mut fresh = inbox("");
    started(&mut f, &mut fresh);
    assert_eq!(take_pending(), [1]);
    take_log();
}
