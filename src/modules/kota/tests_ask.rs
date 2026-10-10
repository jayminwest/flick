//! Quick ask through the module: the verb's later answer, the view, the hotkey and fast
//! polling, over the fakes in `testkit` (ssh target `ok` queues, `down` refuses).

use super::tests::{command, configured, event, wait};
use super::*;
use crate::core::test_cx;

const ASKS: &str = "[kota]\nssh = \"ok\"\nmachine = \"server\"\ndash = \"http://ok\"\n";

/// The ask's final answer, as the control socket waits for it.
fn answer(now: Result<String, String>) -> Result<String, String> {
    later::settle(now, later::take(), later::MAX_WAIT)
}

fn sent(k: &Kota) -> bool {
    k.shared.lock().asks.first().is_some_and(|a| a.status != Status::Sending)
}

#[test]
fn the_verb_answers_once_kota_has_the_question() {
    let mut k = configured(ASKS).unwrap();
    let now = command(&mut k, &["ask", "what's", "on?"], false);
    let id = k.shared.lock().asks[0].id.clone();
    assert_eq!(now, Ok(format!("Asking KOTA ({id})")));
    assert_eq!(answer(now), Ok(format!("Asked KOTA ({id}): queued for KOTA (t1)")));
    wait("settled", || sent(&k));
    let p = k.shared.lock();
    assert_eq!((p.asks[0].text.as_str(), p.asks[0].at), ("what's on?", 1_000));
    assert_eq!(p.asks[0].status, Status::Sent("queued for KOTA (t1)".into()));
    // Fast polling for `fast_secs`; nothing polled before `Started`.
    assert_eq!((p.fast_until, p.started_at), (Some(1_180), None));
}

#[test]
fn a_failed_ask_is_the_verbs_error() {
    let mut k = configured("[kota]\nssh = \"down\"").unwrap();
    let now = command(&mut k, &["ask", "hi"], false);
    assert_eq!(answer(now), Err("kota ask: ssh: connect to host down port 22: Connection refused".into()));
    wait("settled", || sent(&k));
    assert!(matches!(&k.shared.lock().asks[0].status, Status::Failed(why) if why.starts_with("ssh: connect")));
    // A bad question never starts: no later answer either.
    assert_eq!(command(&mut k, &["ask", " "], false), Err("usage: flick kota ask <text>".into()));
    assert!(later::take().is_none());
    assert_eq!(command(&mut k, &["ask"], false), Err("usage: flick kota ask <text>".into()));
    assert!(later::take().is_none());
}

#[test]
fn an_ask_polls_at_once_on_demand() {
    let mut k = configured(&format!("{ASKS}poll_secs = 0")).unwrap();
    event(&mut k, Event::Started);
    assert_eq!(k.shared.lock().started_at, None);
    let now = command(&mut k, &["ask", "hi"], false);
    assert!(answer(now).is_ok());
    assert_eq!(k.shared.lock().started_at, Some(1_000));
    wait("round", || !k.shared.lock().running);
}

#[test]
fn an_ask_with_a_timer_makes_the_next_round_fast() {
    let mut k = configured(ASKS).unwrap();
    event(&mut k, Event::Started);
    wait("first round", || {
        let p = k.shared.lock();
        !p.running && p.ended_at.is_some()
    });
    // KOTA idle (not thinking): the next round in `poll_secs`.
    k.shared.lock().presence = presence::Presence::default();
    event(&mut k, Event::ModuleChanged { module: ID });
    assert_eq!(k.shared.lock().timer_due, Some(1_060));
    let now = command(&mut k, &["ask", "hi"], false);
    assert!(answer(now).is_ok());
    assert_eq!(k.shared.lock().timer_due, Some(1_015));
}

#[test]
fn the_hotkey_opens_the_ask_view() {
    let mut k = configured("[kota]\nhotkey = \"cmd+shift+K\"").unwrap();
    let bound = k.hotkeys();
    assert_eq!(bound, [Binding { spec: "cmd+shift+K".into(), key: Ok("ask".into()) }]);
    assert!(configured("[kota]\nhotkey = \" \"").unwrap().hotkeys().is_empty());
    assert!(configured("").unwrap().hotkeys().is_empty());
    let view = test_cx("", |cx| k.hotkey("ask", cx)).unwrap();
    assert!(view.is(ID, "ask") && view.escape_hides);
    assert_eq!(view.placeholder, "Ask KOTA…");
    // The view polls fast while it shows.
    assert_eq!(k.shared.lock().fast_until, Some(1_000 + VIEW_FAST_SECS));
    assert!(test_cx("", |cx| k.hotkey("nope", cx)).is_none());
    assert!(test_cx("", |cx| k.open("ask", cx)).is_some());
    assert!(test_cx("", |cx| k.open("nope", cx)).is_none());
}

#[test]
fn the_view_sends_the_typed_question() {
    let mut k = configured(ASKS).unwrap();
    let mut view = test_cx("", |cx| k.open("ask", cx)).unwrap();
    test_cx("  ", |cx| k.refresh(&mut view, cx));
    assert!(view.items.is_empty());
    assert_eq!((view.footer.as_str(), view.text.as_str()), ("KOTA: unknown  ·  ↵ sends", ""));
    test_cx(" what's on? ", |cx| k.refresh(&mut view, cx));
    assert_eq!(view.items.len(), 1);
    let item = view.items[0].clone();
    assert_eq!((item.title.as_str(), item.id.arg()), ("Ask KOTA: what's on?", Some("what's on?")));
    assert!(matches!(test_cx("", |cx| k.activate(&item.id, cx)), Outcome::Hide));
    assert!(later::take().is_none(), "the launcher does not wait");
    wait("settled", || sent(&k));
    test_cx("", |cx| k.refresh(&mut view, cx));
    assert_eq!(view.text, "00:16  sent  what's on?");
    // A blank question stays with its error; other ids do nothing.
    let blank = ItemId::new(ID, ASK).with_arg(" ");
    assert!(matches!(test_cx("", |cx| k.activate(&blank, cx)), Outcome::Stay(Some(e)) if e.starts_with("usage")));
    assert!(matches!(test_cx("", |cx| k.activate(&ItemId::new(ID, "x"), cx)), Outcome::Stay(None)));
    // Another view's refresh is left alone.
    let mut other = ListView::new(ID, "other");
    test_cx("hi", |cx| k.refresh(&mut other, cx));
    assert!(other.items.is_empty() && other.footer.is_empty());
}
