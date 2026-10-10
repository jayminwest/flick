//! The module over fake children (`testkit`): never the real herdr or curl.

use super::*;
use crate::config::parse;
use crate::core::test_cx;
use presence::State;
use std::thread;
use std::time::{Duration, Instant};
use testkit::HOOKS;

fn configured(text: &str) -> Result<Kota, String> {
    let mut k = Kota::with_hooks(HOOKS);
    k.configure(&parse(text)?.section(ID)?.ok_or("disabled")?)?;
    Ok(k)
}

fn event(k: &mut Kota, e: Event) {
    test_cx("", |cx| assert!(!k.on_event(e, cx)));
}

fn command(k: &mut Kota, words: &[&str], json: bool) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    test_cx("", |cx| {
        cx.json = json;
        k.command(&args, cx)
    })
}

fn wait(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

const SERVER: &str = "[kota]\nmachine = \"server\"\ndash = \"http://ok\"\n";

#[test]
fn without_a_kota_key_nothing_runs() {
    let mut k = configured("").unwrap();
    assert!(!k.active);
    for e in [Event::Started, Event::ModuleChanged { module: ID }, Event::Wake, Event::LauncherOpened] {
        event(&mut k, e);
    }
    let p = k.shared.lock();
    assert_eq!((p.started_at, p.timer_due, p.running), (None, None, false));
    drop(p);
    let status = command(&mut k, &["status"], false).unwrap();
    assert_eq!(status, "KOTA: unknown\nNot checked yet\npolling: off (set a key in [kota] to poll)");
    // `[kota]` with only `enabled = true` is the same as none.
    assert!(!configured("[kota]\nenabled = true").unwrap().active);
}

#[test]
fn nothing_runs_before_started() {
    let mut k = configured(SERVER).unwrap();
    assert!(k.active);
    event(&mut k, Event::ModuleChanged { module: ID });
    event(&mut k, Event::Wake);
    assert_eq!(k.shared.lock().started_at, None);
}

#[test]
fn started_polls_and_status_reports_it() {
    let mut k = configured(SERVER).unwrap();
    event(&mut k, Event::Started);
    wait("first round", || k.shared.lock().ended_at.is_some());
    event(&mut k, Event::ModuleChanged { module: ID });
    {
        let p = k.shared.lock();
        assert!(p.transitions.is_empty(), "drained");
        // KOTA thinks: the next round in 15 s.
        assert_eq!(p.timer_due, Some(1_015));
    }
    let json: serde_json::Value = serde_json::from_str(&command(&mut k, &["status"], true).unwrap()).unwrap();
    assert_eq!(json["state"], "thinking");
    assert_eq!(json["pane"]["title"], "Flick KOTA cards design brainstorm");
    assert_eq!((json["since"].as_u64(), json["checked_at"].as_u64()), (Some(1_000), Some(1_000)));
    assert_eq!((json["pending"].as_u64(), json["polling"].as_str()), (Some(0), Some("every 60 s")));
    let text = command(&mut k, &["status"], false).unwrap();
    assert!(text.starts_with("KOTA: thinking · <1m\nFlick KOTA cards design brainstorm\nChecked 00:16"), "{text}");
    assert_eq!(command(&mut k, &["refresh"], false).unwrap(), "kota: checked 0 s ago; refresh again in 10 s");
}

#[test]
fn sleep_marks_stale_and_wake_arms_a_round() {
    let mut k = configured(SERVER).unwrap();
    event(&mut k, Event::Started);
    wait("first round", || {
        let p = k.shared.lock();
        !p.running && p.ended_at.is_some()
    });
    event(&mut k, Event::Locked);
    {
        let p = k.shared.lock();
        assert!(p.asleep && p.presence.stale && p.timer_due.is_none());
    }
    assert!(command(&mut k, &["status"], false).unwrap().contains("(stale)\n"));
    event(&mut k, Event::Unlocked);
    let p = k.shared.lock();
    assert!(!p.asleep);
    assert_eq!(p.timer_due, Some(1_000 + io::WAKE_DELAY));
    drop(p);
    event(&mut k, Event::Sleep);
    event(&mut k, Event::Wake);
    assert_eq!(k.shared.lock().woke_at, Some(1_000));
}

#[test]
fn on_demand_polls_on_launcher_open_only() {
    let mut k = configured("[kota]\npoll_secs = 0\nmachine = \"server\"\ndash = \"http://ok\"").unwrap();
    event(&mut k, Event::Started);
    assert_eq!(k.shared.lock().started_at, None);
    assert!(command(&mut k, &["status"], false).unwrap().ends_with("polling: on demand"));
    event(&mut k, Event::LauncherOpened);
    assert_eq!(k.shared.lock().started_at, Some(1_000));
    wait("round", || !k.shared.lock().running);
    event(&mut k, Event::ModuleChanged { module: ID });
    assert_eq!(k.shared.lock().timer_due, None);
}

#[test]
fn reload_resets_on_a_new_target_and_stops_when_emptied() {
    let mut k = configured(SERVER).unwrap();
    event(&mut k, Event::Started);
    wait("first round", || {
        let p = k.shared.lock();
        !p.running && p.ended_at.is_some()
    });
    let epoch = k.shared.lock().epoch;
    // Same target, new interval: the presence stays.
    k.configure(&parse(&format!("{SERVER}poll_secs = 120")).unwrap().section(ID).unwrap().unwrap()).unwrap();
    assert_eq!(k.shared.lock().presence.state, State::Thinking);
    assert_eq!(k.shared.lock().epoch, epoch);
    // A new machine: forget it and check again at once.
    k.configure(&parse("[kota]\nmachine = \"empty\"\ndash = \"http://ok\"").unwrap().section(ID).unwrap().unwrap()).unwrap();
    assert_eq!(k.shared.lock().epoch, epoch + 1);
    wait("new round", || k.shared.lock().ended_at.is_some());
    wait("new round end", || !k.shared.lock().running);
    // No key left: polling stops; the module stays for `kota status`.
    k.configure(&parse("").unwrap().section(ID).unwrap().unwrap()).unwrap();
    assert!(!k.active);
    assert!(k.shared.lock().timer_due.is_none());
    assert!(command(&mut k, &["status"], false).unwrap().ends_with("polling: off (set a key in [kota] to poll)"));
    // And a key again starts it.
    k.configure(&parse("[kota]\nmachine = \"empty\"\ndash = \"http://ok\"").unwrap().section(ID).unwrap().unwrap()).unwrap();
    assert!(k.active);
    let p = k.shared.lock();
    assert!(p.timer_due.is_some() || p.running);
    drop(p);
    // A bad table changes nothing.
    assert!(k.configure(&parse("[kota]\npoll_secs = 1").unwrap().section(ID).unwrap().unwrap()).is_err());
}

#[test]
fn verbs_and_unknown_commands() {
    let mut k = configured("").unwrap();
    assert_eq!(k.id(), "kota");
    assert_eq!(k.verbs(), "kota status | kota refresh");
    assert_eq!(command(&mut k, &["nope"], false), Err("kota: unknown command \"nope\"".into()));
    assert_eq!(command(&mut k, &["status", "x"], false), Err("kota: unknown command \"status\"".into()));
    assert_eq!(command(&mut k, &[], false), Err("kota: missing command".into()));
    assert_eq!(Kota::default().settings, Settings::default());
    assert!(unix_now() > 1_700_000_000);
}
