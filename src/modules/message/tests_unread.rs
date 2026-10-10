//! Unread posts nobody asked for (flick-cb7d, `seen.rs`): which posts are unread, how long
//! their cards stay, what reads them, and the unread count in `Event::CardsPending`.

use super::dispatch::Note;
use super::seen::Seen;
use super::tests::{Fixture, inbox, queue, take_log, take_pending, take_unread};
use super::*;

fn started(f: &mut Fixture, m: &mut Inbox) {
    f.cx("", false, |cx| m.on_event(Event::Started, cx));
}

fn drain(f: &mut Fixture, m: &mut Inbox) {
    f.cx("", false, |cx| m.on_event(Event::ModuleChanged { module: "message" }, cx));
}

/// The card timeout of the last `show` the fakes logged.
fn timeout_of(log: &[String]) -> &str {
    log.last().and_then(|l| l.rsplit('|').nth(2)).unwrap_or("")
}

#[test]
fn only_a_post_nobody_asked_for_is_unread_and_stays() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    started(&mut f, &mut m);
    assert_eq!((take_pending(), take_unread()), (vec![0], vec![0]));
    f.run(&mut m, false, &["post", "--id", "n1", "Off the laptop?"]).unwrap();
    assert_eq!(timeout_of(&take_log()), "0", "it stays until dismissed");
    assert_eq!((take_pending(), take_unread()), (vec![0], vec![1]));
    // A reply, a placeholder and a chat post time out as before and are not unread.
    f.run(&mut m, false, &["post", "--id", "p", "--pending", "q?"]).unwrap();
    assert_eq!(timeout_of(&take_log()), "20");
    f.run(&mut m, false, &["post", "--id", "r", "--reply-to", "p", "a"]).unwrap();
    assert_eq!(timeout_of(&take_log()), "20");
    f.run(&mut m, false, &["post", "--thread", "t", "--id", "c", "chat"]).unwrap();
    take_log();
    assert_eq!(take_unread(), [0; 0]);
    let unread: Vec<_> = f.store.messages(10).into_iter().filter(|m| m.unread).map(|m| m.id).collect();
    assert_eq!(unread, ["n1"]);
    // `show` re-shows it with the same rule.
    f.run(&mut m, false, &["show", "n1"]).unwrap();
    assert_eq!(timeout_of(&take_log()), "0");
}

#[test]
fn closing_its_card_reads_it_but_a_timeout_does_not() {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nunprompted_timeout_secs = 45"));
    f.run(&mut m, false, &["post", "--id", "n1", "one"]).unwrap();
    f.run(&mut m, false, &["post", "--id", "n2", "two"]).unwrap();
    assert_eq!(timeout_of(&take_log()), "45");
    assert_eq!(take_unread(), [1, 2]);
    queue(Note::Expired("n1".into()));
    drain(&mut f, &mut m);
    assert_eq!(take_unread(), [0; 0], "still unread");
    queue(Note::Dismissed("n1".into()));
    drain(&mut f, &mut m);
    assert_eq!(take_unread(), [1]);
    assert!(!f.store.message("n1").unwrap().unread);
    // A restart keeps the rest unread.
    let mut fresh = inbox("");
    started(&mut f, &mut fresh);
    assert_eq!(take_unread(), [1]);
    // `hide` leaves it unread.
    f.run(&mut fresh, false, &["hide"]).unwrap();
    assert_eq!(f.store.unread_count(), 1);
    take_log();
    take_pending();
}

#[test]
fn opening_the_list_reads_everything_and_closes_those_cards() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    f.run(&mut m, false, &["post", "--id", "n1", "one"]).unwrap();
    f.run(&mut m, false, &["post", "--id", "n2", "two"]).unwrap();
    queue(Note::Dismissed("n1".into()));
    drain(&mut f, &mut m);
    take_log();
    take_unread();
    let mut view = f.cx("", false, |cx| m.open(RECENT, cx)).unwrap();
    assert_eq!(take_log(), ["dismiss n2"]);
    assert_eq!(take_unread(), [0]);
    f.cx("", false, |cx| m.refresh(&mut view, cx));
    let marks: Vec<_> = view.items.iter().map(|i| (i.id.key(), i.accessory.as_str())).collect();
    assert_eq!(marks, [("n2", "Unread"), ("n1", "")]);
    // A post that arrives while the list shows is unread there too.
    f.run(&mut m, false, &["post", "--id", "n3", "three"]).unwrap();
    f.cx("", false, |cx| m.refresh(&mut view, cx));
    assert_eq!(view.items[0].accessory, "Unread");
    assert_eq!(f.run(&mut m, false, &["ls"]).unwrap().lines().next().map(|l| l.ends_with("three (unread)")), Some(true));
    // Each opening marks only what it read; then the rows show what they did before.
    let marked = |f: &mut Fixture, m: &mut Inbox| {
        let mut view = f.cx("", false, |cx| m.open(RECENT, cx)).unwrap();
        f.cx("", false, |cx| m.refresh(&mut view, cx));
        view.items.iter().filter(|i| i.accessory == "Unread").map(|i| i.id.key().to_string()).collect::<Vec<_>>()
    };
    assert_eq!(marked(&mut f, &mut m), ["n3"]);
    assert_eq!(marked(&mut f, &mut m), [""; 0]);
    take_log();
    take_pending();
    take_unread();
}
