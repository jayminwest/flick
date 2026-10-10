//! Local card actions (`dispatch.rs`, flick-e244) and `message card press` over the shared
//! fixture. Opening links and apps, the pasteboard, this binary's path and every local run
//! are fakes (`tests::inbox`): nothing opens, nothing touches the clipboard, nothing spawns.

use serde_json::json;

use super::dispatch::{NO_COMMAND, Pressed, runnable};
use super::tests::{Fixture, inbox, take_log};
use super::tests_press::{posted, press, settle};
use super::*;
use crate::core::card::{Action, Do, Kind, Origin, Style};

const EXE: &str = "/Flick.app/Contents/MacOS/Flick";

/// A card `c1` with these actions, posted from this Mac (`local`) or a peer.
fn post(actions: &serde_json::Value, local: bool, config: &str) -> (Fixture, Inbox) {
    let (mut f, mut m) = (Fixture::new(), inbox(config));
    f.local = local;
    let card = json!({ "id": "c1", "title": "T", "actions": actions }).to_string();
    f.run(&mut m, false, &["card", "post", &card]).unwrap();
    take_log();
    (f, m)
}

fn cli(f: &mut Fixture, m: &mut Inbox, words: &[&str]) -> Result<String, String> {
    f.local = true;
    let words: Vec<&str> = ["card", "press"].iter().chain(words).copied().collect();
    f.run(m, false, &words)
}

#[test]
fn open_url_open_app_and_copy_run_at_once_and_say_so() {
    let actions = json!([
        { "id": "web", "label": "Open", "do": { "open_url": "https://example.com/pr/1" } },
        { "id": "app", "label": "Safari", "do": { "open_app": "com.apple.Safari" } },
        { "id": "nope", "label": "Nope", "do": { "open_app": "Nope" } },
        { "id": "cp", "label": "Copy", "do": { "copy": "ship it" } }]);
    let (mut f, mut m) = post(&actions, false, "");
    for (action, did, line) in [
        ("web", "open https://example.com/pr/1", "Opened the link"),
        ("app", "app com.apple.Safari", "Opened com.apple.Safari"),
        ("cp", "copy ship it", "Copied"),
    ] {
        assert_eq!(cli(&mut f, &mut m, &["c1", action]), Ok(line.to_string()));
        assert_eq!(take_log(), [did.to_string(), format!("update c1 Open|false|None|None|{line}|0|true")]);
    }
    // An app that is not there is an error line, and the press's error.
    assert_eq!(cli(&mut f, &mut m, &["c1", "nope"]), Err(r#"open_app: no app "Nope""#.into()));
    let err = r#"Some("open_app: no app \"Nope\"")"#;
    assert_eq!(take_log(), ["app Nope".to_string(), format!("update c1 Open|false|{err}|None|0|true")]);
}

#[test]
fn script_and_flick_self_exec_on_the_worker_with_the_cards_origin() {
    // From a peer: script runs only because the user pressed it here; the run is marked remote.
    let actions = json!([
        { "id": "s", "label": "Deploy", "do": { "script": { "name": "deploy", "query": "prod" } } },
        { "id": "t", "label": "Start", "do": { "flick": ["task", "start", "x"] } }]);
    let (mut f, mut m) = post(&actions, false, "");
    assert_eq!(cli(&mut f, &mut m, &["c1", "s"]), Ok("Running script deploy; the result shows on the card".into()));
    settle(&mut f, &mut m);
    assert_eq!(
        take_log(),
        [
            "update c1 Open|true|None|None|Running script deploy…|0|true".to_string(),
            format!("update c1 Open|false|None|None|ran {EXE} script run deploy prod [remote]|0|true"),
        ]
    );
    // From this Mac: not marked remote.
    let (mut f, mut m) = post(&actions, true, "");
    press("t");
    settle(&mut f, &mut m);
    assert_eq!(take_log()[1], format!("update c1 Open|false|None|None|ran {EXE} task start x|0|true"));
}

#[test]
fn a_failed_run_shows_its_error_and_sends_nothing() {
    let actions = json!([
        { "id": "t", "label": "Fail", "do": { "flick": ["task", "fail"] }, "reply": true }]);
    let (mut f, mut m) = post(&actions, true, "[message]\naction_command = [\"sent\"]");
    press("t");
    settle(&mut f, &mut m);
    let err = r#"Some("flick task fail failed (exit 1): no")"#;
    assert_eq!(take_log()[1], format!("update c1 Open|false|{err}|None|0|true"));
    assert!(m.ui["c1"].error.is_some() && !m.ui["c1"].busy());
}

#[test]
fn a_replying_run_sends_its_press_after_it_works() {
    let actions = json!([
        { "id": "t", "label": "Start", "do": { "flick": ["task", "start", "x"] }, "reply": true }]);
    let (mut f, mut m) = post(&actions, true, "[message]\naction_command = [\"sent\"]");
    press("t");
    settle(&mut f, &mut m);
    assert_eq!(
        take_log(),
        ["update c1 Open|true|None|None|Running flick task start x…|0|true", "update c1 Open|true|None|None|0|true", "wake 121"]
    );
    // Without action_command the reply's error shows once the run is done.
    let (mut f, mut m) = post(&actions, true, "");
    press("t");
    settle(&mut f, &mut m);
    assert_eq!(take_log()[1], format!("update c1 Open|false|Some({NO_COMMAND:?})|None|0|true"));
}

#[test]
fn a_remote_card_cannot_press_a_verb_denied_over_the_network() {
    let actions = json!([
        { "id": "r", "label": "Reload", "do": { "flick": ["reload"] } },
        { "id": "s", "label": "Run", "do": { "flick": ["script", "run", "deploy"] } }]);
    let (mut f, mut m) = post(&actions, false, "");
    for (action, why) in [("r", "flick: reload: not allowed over the network"), ("s", "flick: script run: not allowed over the network")] {
        assert_eq!(cli(&mut f, &mut m, &["c1", action]), Err(why.into()));
        assert_eq!(take_log(), [format!("update c1 Open|false|Some({why:?})|None|20|false")]);
    }
    // The HUD path refuses the same way, and nothing runs.
    press("r");
    settle(&mut f, &mut m);
    assert!(!take_log().iter().any(|l| l.contains("ran ")));
    // The same card from this Mac runs it.
    let (mut f, mut m) = post(&actions, true, "");
    press("r");
    settle(&mut f, &mut m);
    assert_eq!(take_log()[1], format!("update c1 Open|false|None|None|ran {EXE} reload|0|true"));
}

#[test]
fn the_press_time_check_uses_the_stored_origin() {
    // A local card with a verb peers may not send, then marked as posted over the network.
    let actions = json!([{ "id": "r", "label": "Reload", "do": { "flick": ["reload"] } }]);
    let (mut f, mut m) = post(&actions, true, "");
    f.store.conn().execute("UPDATE messages SET remote = 1 WHERE id = 'c1'", []).unwrap();
    let why = "flick: reload: not allowed over the network";
    assert_eq!(cli(&mut f, &mut m, &["c1", "r"]), Err(why.into()));
    // `runnable` re-checks a `do` even when the action came in enabled.
    let action = |run: Do| Action { id: "a".into(), label: "A".into(), style: Style::Default, kind: Kind::Local { run, reply: true } };
    let reload = action(Do::Flick(vec!["reload".into()]));
    assert_eq!(runnable(&reload, Origin::Remote), Err(why.into()));
    assert_eq!(runnable(&reload, Origin::Local), Ok(Some((&Do::Flick(vec!["reload".into()]), true))));
    let reply = Action { kind: Kind::Reply, ..reload.clone() };
    assert_eq!(runnable(&reply, Origin::Remote), Ok(None));
    let disabled = Action { kind: Kind::Disabled { raw: json!(1), reply: false, reason: "no".into() }, ..reload };
    assert_eq!(runnable(&disabled, Origin::Local), Err("no".into()));
}

#[test]
fn no_binary_is_an_error_line() {
    let actions = json!([{ "id": "t", "label": "Start", "do": { "flick": ["task", "ls"] } }]);
    let (mut f, mut m) = post(&actions, true, "");
    m.env.self_exe = || Err("cannot find Flick's binary: gone".into());
    assert_eq!(cli(&mut f, &mut m, &["c1", "t"]), Err("cannot find Flick's binary: gone".into()));
    assert!(m.local.take().is_empty());
}

#[test]
fn a_run_for_a_dismissed_or_reposted_card_is_ignored() {
    let (mut f, mut m) = posted("");
    press("sh");
    press("sh");
    // Dismissed while it runs: the result arrives and changes nothing. The dismissal is
    // queued with the presses so one drain handles all three before any result (a fast
    // run could otherwise finish inside the drain that started it).
    queue_dismiss();
    f.cx("", false, |cx| m.drain(cx));
    settle(&mut f, &mut m);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while m.local.running() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    settle(&mut f, &mut m);
    assert!(m.ui.is_empty(), "{:?}", m.ui);
    assert!(!take_log().iter().any(|l| l.contains("ran ")));
}

fn queue_dismiss() {
    super::tests::queue(dispatch::Note::Dismissed("c1".into()));
}

#[test]
fn card_press_answers_what_happened() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"echo\"]");
    // A shell action: the first press shows the confirm, :cancel cancels, a second press runs.
    let confirm = "Confirm on the card: make deploy\nPress sh again to run it";
    assert_eq!(cli(&mut f, &mut m, &["c1", "sh"]), Ok(confirm.into()));
    assert_eq!(cli(&mut f, &mut m, &["c1", ":cancel"]), Ok("Cancelled".into()));
    assert_eq!(cli(&mut f, &mut m, &["c1", "sh"]), Ok(confirm.into()));
    assert_eq!(cli(&mut f, &mut m, &["c1", "sh"]), Ok("Running command; the result shows on the card".into()));
    // Busy while it runs.
    assert_eq!(cli(&mut f, &mut m, &["c1", "go"]), Err("Card c1 is busy: waiting on KOTA or a run".into()));
}

#[test]
fn card_press_sends_values_and_reports_refusals() {
    let (mut f, mut m) = posted("[message]\naction_command = [\"echo\"]");
    // A reply press sends the card's initial values unless given.
    assert_eq!(cli(&mut f, &mut m, &["c1", "go"]), Ok("Sent go to KOTA; the card waits for its update".into()));
    settle(&mut f, &mut m);
    let log = take_log();
    assert!(log[1].contains(r#"--action-id go <{\"why\":\"\"}"#), "{log:?}");
    cli(&mut f, &mut m, &["c1", "go", r#"{"why":"now"}"#]).unwrap();
    settle(&mut f, &mut m);
    assert!(take_log()[1].contains(r#"<{\"why\":\"now\"}"#));
    for bad in ["[1]", "nope"] {
        assert_eq!(cli(&mut f, &mut m, &["c1", "go", bad]), Err("values must be a JSON object".into()));
    }
    let big = json!({ "why": "x".repeat(4000) }).to_string();
    assert_eq!(cli(&mut f, &mut m, &["c1", "go", &big]), Err("values over 4000 chars".into()));
    assert_eq!(cli(&mut f, &mut m, &["c1", "later"]), Ok("Closed c1".into()));
    assert_eq!(cli(&mut f, &mut m, &["c9", "go"]), Err("No card c9".into()));
    assert_eq!(cli(&mut f, &mut m, &["c1", "zzz"]), Err("No action zzz on card c1".into()));
    assert!(cli(&mut f, &mut m, &["c1"]).unwrap_err().starts_with("usage: flick message card"));
    assert!(cli(&mut f, &mut m, &["c1", "go", "{}", "x"]).is_err());
}

#[test]
fn a_card_press_is_the_same_press_as_the_huds() {
    let (mut f, mut m) = posted("");
    assert_eq!(cli(&mut f, &mut m, &["c1", "go"]), Err(NO_COMMAND.into()));
    let pressed = f.cx("", false, |cx| m.press("c1", "web", "{}".into(), cx));
    assert_eq!(pressed, Ok(Pressed::Did("Opened the link".into())));
}
