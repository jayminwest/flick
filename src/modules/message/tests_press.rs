//! Card presses and dismissals (`dispatch.rs`) over the shared fixture: KOTA sends through a
//! fake `exec` on the real worker thread, the pending state and its watchdog, local actions
//! and the shell confirm. The HUD handlers are faked by `queue`; nothing reaches `AppKit`.

use std::thread;
use std::time::{Duration, Instant};

use super::dispatch::{LOCAL_LATER, NO_COMMAND, NO_UPDATE, Note, Phase};
use super::tests::{Fixture, TS, inbox, queue, take_log};
use super::*;
use crate::platform::hud::CANCEL;

const ASK: &str = r#"{"id":"c1","title":"Deploy?","blocks":[{"type":"field","id":"why","label":"Why"}],"actions":[
  {"id":"go","label":"Ship","style":"primary"},
  {"id":"later","label":"Later","do":"dismiss"},
  {"id":"bye","label":"Bye","do":"dismiss","reply":true},
  {"id":"web","label":"Open","do":{"open_url":"https://example.com"}},
  {"id":"web2","label":"Open+","do":{"open_url":"https://example.com"},"reply":true},
  {"id":"sh","label":"Run","do":{"shell":"make deploy"}}]}"#;

fn press(action: &str) {
    queue(Note::Press { card: "c1".into(), action: action.into(), values: r#"{"why":"ok"}"#.into() });
}

/// Handle queued notes and finished sends until no send runs.
fn settle(f: &mut Fixture, m: &mut Inbox) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        f.cx("", false, |cx| m.on_event(Event::ModuleChanged { module: "message" }, cx));
        let sending = m.ui.values().any(|u| matches!(u.phase, Phase::Sending { .. }));
        if !sending || Instant::now() > deadline {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn posted(config: &str) -> (Fixture, Inbox) {
    let (mut f, mut m) = (Fixture::new(), inbox(config));
    f.run(&mut m, false, &["card", "post", ASK]).unwrap();
    take_log();
    (f, m)
}

#[test]
fn a_reply_press_without_action_command_shows_an_error() {
    let (mut f, mut m) = posted("");
    press("go");
    settle(&mut f, &mut m);
    assert_eq!(take_log(), [format!("update c1 Open|false|Some({NO_COMMAND:?})|None|0|true")]);
}

#[test]
fn a_sent_press_waits_for_kota_and_a_repost_clears_it() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]");
    press("go");
    settle(&mut f, &mut m);
    // Pending while sending (no timeout), then waiting for KOTA with the watchdog armed.
    assert_eq!(take_log(), ["update c1 Open|true|None|None|0|true", "wake 121"]);
    assert_eq!(m.ui["c1"].phase, Phase::Waiting { since: TS });
    // Presses on a pending card are ignored.
    press("go");
    settle(&mut f, &mut m);
    assert_eq!(take_log(), [""; 0]);
    // KOTA re-posts the card: the press state is gone.
    f.run(&mut m, false, &["card", "post", ASK]).unwrap();
    assert_eq!(take_log(), ["card c1 Deploy? Open|false|None|None|TopRight|4|0|true|true"]);
    assert!(m.ui.is_empty());
}

#[test]
fn the_watchdog_turns_a_silent_kota_into_an_error() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]\npending_timeout_secs = 30");
    press("go");
    settle(&mut f, &mut m);
    assert_eq!(take_log(), ["update c1 Open|true|None|None|0|true", "wake 31"]);
    m.env.now = || TS + 29;
    settle(&mut f, &mut m);
    assert_eq!(take_log(), [""; 0]);
    m.env.now = || TS + 30;
    settle(&mut f, &mut m);
    assert_eq!(take_log(), [format!("update c1 Open|false|Some({NO_UPDATE:?})|None|0|true")]);
    // The actions are back: the user may press again.
    press("go");
    settle(&mut f, &mut m);
    assert_eq!(take_log()[0], "update c1 Open|true|None|None|0|true");
}

#[test]
fn no_watchdog_when_its_timeout_is_zero() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]\npending_timeout_secs = 0");
    press("go");
    settle(&mut f, &mut m);
    assert_eq!(take_log(), ["update c1 Open|true|None|None|0|true"]);
    m.env.now = || TS + 100_000;
    settle(&mut f, &mut m);
    assert_eq!(take_log(), [""; 0]);
}

#[test]
fn rejections_and_failures_show_on_the_card_with_actions_on() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"reject\"]");
    press("go");
    settle(&mut f, &mut m);
    let log = take_log();
    assert_eq!(log[1], "update c1 Open|false|Some(\"KOTA rejected: card is gone\")|None|0|true");
    // The argv gets the card and action; the values go on stdin.
    let (mut f, mut m) = posted("[message]\naction_command = [\"echo\", \"-x\"]");
    press("go");
    settle(&mut f, &mut m);
    let expect = r#"echo -x --action --card c1 --action-id go <{"why":"ok"}"#;
    assert_eq!(take_log()[1], format!("update c1 Open|false|Some({expect:?})|None|0|true"));
    // A replying local action sends its press too.
    press("web2");
    settle(&mut f, &mut m);
    assert!(take_log()[1].contains("--action-id web2"));
}

#[test]
fn a_result_for_a_reposted_or_dismissed_card_is_ignored() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]");
    press("go");
    // Re-posted before the worker's result is handled.
    f.cx("", false, |cx| m.drain(cx));
    f.run(&mut m, false, &["card", "post", ASK]).unwrap();
    settle(&mut f, &mut m);
    thread::sleep(Duration::from_millis(50));
    settle(&mut f, &mut m);
    assert!(m.ui.is_empty(), "{:?}", m.ui);
    assert!(!take_log().iter().any(|l| l.starts_with("wake")));
    // A result for an older press of a card that was pressed again is ignored too.
    press("go");
    f.cx("", false, |cx| m.drain(cx));
    m.ui.get_mut("c1").unwrap().phase = Phase::Sending { press: 99 };
    let deadline = Instant::now() + Duration::from_secs(5);
    while m.worker.len() == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    f.cx("", false, |cx| m.drain(cx));
    assert_eq!(m.ui["c1"].phase, Phase::Sending { press: 99 });
    assert!(!take_log().iter().any(|l| l.starts_with("wake")));
}

#[test]
fn hud_dismissals_mark_the_card_and_drop_its_state() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]");
    press("go");
    settle(&mut f, &mut m);
    queue(Note::Dismissed("c1".into()));
    settle(&mut f, &mut m);
    assert!(m.ui.is_empty() && m.dismissed.contains("c1"));
    take_log();
    // A done update of a card the user closed stays in history (plan risk 10).
    let done = r#"{"id":"c1","title":"Deploy?","state":"done"}"#;
    f.run(&mut m, false, &["card", "post", done]).unwrap();
    assert_eq!(take_log(), [""; 0]);
}

#[test]
fn dismiss_closes_and_local_actions_wait_for_e244() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]");
    press("web");
    settle(&mut f, &mut m);
    assert_eq!(take_log(), [format!("update c1 Open|false|Some({LOCAL_LATER:?})|None|0|true")]);
    press("later");
    settle(&mut f, &mut m);
    assert_eq!(take_log(), ["dismiss c1"]);
    assert!(m.dismissed.contains("c1") && m.ui.is_empty());
    // A dismiss that replies closes the card once the send succeeds.
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]");
    press("bye");
    settle(&mut f, &mut m);
    assert_eq!(take_log(), ["update c1 Open|true|None|None|0|true", "dismiss c1"]);
    assert!(m.dismissed.contains("c1") && m.ui.is_empty());
}

#[test]
fn a_shell_action_asks_first() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]");
    press("sh");
    settle(&mut f, &mut m);
    assert_eq!(take_log(), ["update c1 Open|false|None|Some(\"sh\")|0|true"]);
    press(CANCEL);
    settle(&mut f, &mut m);
    assert_eq!(take_log(), ["update c1 Open|false|None|None|0|true"]);
    press("sh");
    press("sh");
    settle(&mut f, &mut m);
    let log = take_log();
    assert_eq!(log[1], format!("update c1 Open|false|Some({LOCAL_LATER:?})|None|0|true"));
}

#[test]
fn stray_presses_do_nothing() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"sent\"]");
    for (card, action) in [("nope", "go"), ("c1", "nope")] {
        queue(Note::Press { card: card.into(), action: action.into(), values: "{}".into() });
    }
    settle(&mut f, &mut m);
    assert_eq!(take_log(), [""; 0]);
    // A card KOTA marked pending takes no presses.
    let pending = ASK.replacen(r#""title":"Deploy?","#, r#""title":"Deploy?","state":"pending","#, 1);
    f.run(&mut m, false, &["card", "post", &pending]).unwrap();
    take_log();
    press("go");
    settle(&mut f, &mut m);
    assert_eq!(take_log(), [""; 0]);
    // Other events do nothing.
    assert!(!f.cx("", false, |cx| m.on_event(Event::Wake, cx)));
}

#[test]
fn card_timeout_and_action_command_settings() {
    let (_f, m) = posted("[message]\ncard_timeout_secs = 90");
    assert_eq!(m.settings.card_timeout_secs, 90);
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\ncard_timeout_secs = 90"));
    f.run(&mut m, false, &["card", "post", ASK]).unwrap();
    assert!(take_log()[0].ends_with("|90|true|true"), "open action cards use card_timeout_secs");
    let mut fresh = inbox("");
    let bad = crate::config::parse("[message]\naction_command = [\" \"]").unwrap();
    let err = fresh.configure(&bad.section("message").unwrap().unwrap()).unwrap_err();
    assert_eq!(err, "[message]: action_command needs a program first");
}
