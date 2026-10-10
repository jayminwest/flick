//! Tests for module `keys`.

use super::action::tests::{listener, read_when_written, serve, temp};
use super::wire::tests::{HOTKEYS, REMAP_SET, SECURE, STUB, calls, run_later};
use super::*;
use crate::config::parse;
use crate::core::test_cx;
use crate::core::keys::{HYPER_FLAGS, flags};

thread_local! {
    /// The requests `fake_local` was handed, in order.
    static LOCAL: std::cell::RefCell<Vec<Vec<String>>> = const { std::cell::RefCell::new(vec![]) };
}

/// The `Local` hook in tests: records the request instead of running it.
fn fake_local(words: Vec<String>) {
    LOCAL.with(|l| l.borrow_mut().push(words));
}

fn sent() -> Vec<String> {
    LOCAL.with(|l| l.borrow_mut().drain(..).map(|w| w.join(" ")).collect())
}

fn configured(text: &str) -> Result<Keys, String> {
    let mut keys = Keys::new(fake_local);
    keys.wire = Wire::with(&STUB);
    keys.configure(&parse(text)?.section("keys")?.ok_or("disabled")?)?;
    Ok(keys)
}

fn run(keys: &mut Keys, words: &[&str]) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|s| (*s).to_string()).collect();
    test_cx("", |cx| keys.command(&args, cx))
}

const PTT: &str = r#"
    [[keys.chord]]
    name = "ptt"
    keys = ["right_cmd", "right_alt"]
    on_down = { http = "POST http://127.0.0.1:PORT/pipeline/listen/start" }
    on_up = { http = "POST http://127.0.0.1:PORT/pipeline/listen/stop" }
"#;

#[test]
fn off_without_config() {
    let mut keys = configured("").unwrap();
    assert!(keys.off() && keys.worker.idle());
    assert_eq!(run(&mut keys, &["status"]).unwrap(), "keys: off");
    assert_eq!(run(&mut keys, &["list"]).unwrap(), "");
    assert!(test_cx("", |cx| keys.items(cx)).is_empty());
    assert!(configured("[keys]\nhyper = \" \"").unwrap().off());
}

#[test]
fn reads_chords_and_hyper() {
    let text = format!("[keys]\nhyper = \"caps_lock\"\n{PTT}").replace("PORT", "8600");
    let mut keys = configured(&text).unwrap();
    REMAP_SET.with(|r| *r.borrow_mut() = Ok(true));
    let hyper = Hyper {
        key: "caps_lock".into(),
        mods: "cmd+ctrl+alt+shift".into(),
        tap: Some("Escape".into()),
        tap_ms: 300,
    };
    assert_eq!(keys.hyper, Some(hyper));
    assert_eq!(
        run(&mut keys, &["list"]).unwrap(),
        "0\tptt\tright_cmd+right_alt\t\
         down: POST http://127.0.0.1:8600/pipeline/listen/start\t\
         up: POST http://127.0.0.1:8600/pipeline/listen/stop\n\
         hyper\tcaps_lock\tcmd+ctrl+alt+shift\ttap: Escape 300 ms"
    );
    assert_eq!(
        run(&mut keys, &["status"]).unwrap(),
        "tap: Accessibility needed\nsecure input: off\nchords: 1\n\
         hyper: caps_lock (cmd+ctrl+alt+shift)\ncaps lock remap: set"
    );
    REMAP_SET.with(|r| *r.borrow_mut() = Ok(false));
    let items = test_cx("", |cx| keys.items(cx));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "keys:status");
    assert_eq!(items[0].subtitle, "1 chord · hyper on caps_lock · Accessibility needed");
    let out = test_cx("", |cx| keys.activate(&items[0].id, cx));
    assert!(matches!(out, Outcome::Stay(Some(s)) if s == items[0].subtitle));
    let other = test_cx("", |cx| keys.activate(&ItemId::new("keys", "x"), cx));
    assert!(matches!(other, Outcome::Stay(None)));

    let text = "[keys]\nhyper = \"F18\"\nhyper_tap = \"\"\nhyper_tap_ms = 200";
    let keys = configured(text).unwrap();
    let h = keys.hyper.as_ref().unwrap();
    assert_eq!((h.tap.as_deref(), h.tap_ms), (None, 200));
    assert_eq!(keys.list(), "hyper\tF18\tcmd+ctrl+alt+shift\ttap: - 200 ms");
}

#[test]
fn secure_input_pauses_hyper() {
    let keys = configured("[keys]\nhyper = \"F18\"").unwrap();
    assert_eq!(keys.summary(), "0 chords · hyper on F18 · Accessibility needed");
    assert!(!keys.remap);
    SECURE.with(|s| s.replace(true));
    assert_eq!(keys.summary(), "0 chords · hyper on F18 · Secure input is on: hyper paused");
    assert!(keys.status().contains("secure input: on"));
    SECURE.with(|s| s.replace(false));
}

#[test]
fn compiles_rules_for_the_tap() {
    let text = format!("[keys]\nhyper = \"caps_lock\"\n{PTT}").replace("PORT", "8600");
    let keys = configured(&text).unwrap();
    // caps_lock compiles to F18 (the HID remap's target); chords keep their order.
    let rule = keys::Hyper { source: F18, flags: HYPER_FLAGS, tap: Some(53), tap_ms: 300 };
    let mods = flags::CMD | flags::RIGHT_CMD | flags::ALT | flags::RIGHT_ALT;
    let chord = keys::Chord { mods, key: None };
    assert_eq!(keys.rules, Rules { hyper: Some(rule), chords: vec![chord] });
    let keys = configured("[keys]\nhyper = \"F18\"\nhyper_tap = \"\"").unwrap();
    assert_eq!(keys.rules.hyper.map(|h| (h.source, h.tap)), Some((F18, None)));
    let keys = configured("[keys]\nhyper = \"KeyH\"\nhyper_mods = \"cmd\"").unwrap();
    assert_eq!(keys.rules.hyper.map(|h| (h.source, h.flags)), Some((4, flags::CMD)));
}

#[test]
fn bad_tables_name_the_problem() {
    let chord = |body: &str| format!("[[keys.chord]]\n{body}");
    let cases = [
        (chord("name = \"a\"\nkeys = [\"fn\"]\non_dwn = { shell = \"x\" }"), "unknown field"),
        (chord("name = \"a\"\nkeys = [\"fn\"]\non_up = { exec = \"x\" }"), "unknown variant"),
        (chord("name = \" \"\nkeys = [\"fn\"]"), "[keys] chord: empty name"),
        (chord("name = \"a\"\nkeys = []"), "[keys] chord \"a\": keys: a chord needs at least"),
        (chord("name = \"a\"\nkeys = [\"right_cmd\", \"KeyQ\", \"KeyW\"]"), "at most one"),
        (chord("name = \"a\"\nkeys = [\"hyperspace\"]"), "unknown key name `hyperspace`"),
        (
            chord("name = \"a\"\nkeys = [\"fn\"]\non_down = { http = \"https://x/\" }"),
            "[keys] chord \"a\": http \"https://x/\": only http://",
        ),
        (
            chord("name = \"a\"\nkeys = [\"fn\"]\n") + &chord("name = \"a\"\nkeys = [\"fn\"]"),
            "[keys] chord \"a\": duplicate name",
        ),
        ("[keys]\nhyper = \"F18\"\nhyper_mods = \"\"".into(), "[keys] hyper_mods: unknown"),
        ("[keys]\nhyper = \"F18\"\nhyper_mods = \"cmd+KeyA\"".into(), "not a modifier"),
        ("[keys]\nhyper = \"right_cmd\"".into(), "`right_cmd` is a modifier"),
        ("[keys]\nhyper = \"Hyper\"".into(), "[keys] hyper: unknown key name"),
        ("[keys]\nhyper = \"F18\"\nhyper_tap = \"cmd\"".into(), "hyper_tap: unknown key"),
    ];
    for (text, want) in cases {
        let err = configured(&text).err().unwrap();
        assert!(err.contains(want), "{text}\n=> {err}");
    }
    // Without hyper, hyper_mods is not checked.
    assert!(configured("[keys]\nhyper_mods = \"\"").unwrap().off());
}

#[test]
fn fire_queues_the_edge_in_order() {
    let (l, port) = listener();
    let mut keys = configured(&PTT.replace("PORT", &port.to_string())).unwrap();
    let start = format!("ptt down: queued POST http://127.0.0.1:{port}/pipeline/listen/start");
    assert_eq!(run(&mut keys, &["fire", "ptt", "down"]).unwrap(), start);
    assert!(run(&mut keys, &["fire", "ptt", "up"]).unwrap().starts_with("ptt up: queued"));
    let heads = serve(&l, 2, "200 OK");
    let lines: Vec<_> = heads.iter().map(|h| h.lines().next().unwrap()).collect();
    assert_eq!(lines, ["POST /pipeline/listen/start HTTP/1.1", "POST /pipeline/listen/stop HTTP/1.1"]);

    assert_eq!(run(&mut keys, &["fire", "nope", "up"]).unwrap_err(), "keys: no chord \"nope\"");
    let err = run(&mut keys, &["fire", "ptt", "sideways"]).unwrap_err();
    assert_eq!(err, "keys: fire ptt: expected down or up, got \"sideways\"");
    assert_eq!(keys.fire(5, true).unwrap_err(), "keys: no chord 5");
    assert_eq!(run(&mut keys, &["fire"]).unwrap_err(), "keys: unknown command \"fire\"");
    assert_eq!(run(&mut keys, &[]).unwrap_err(), "keys: missing command");
}

#[test]
fn fire_runs_shell_and_skips_a_missing_edge() {
    let out = temp("fire");
    let text = format!(
        "[[keys.chord]]\nname = \"note\"\nkeys = [\"right_shift\"]\n\
         on_down = {{ shell = \"echo $FLICK_CHORD $FLICK_CHORD_STATE >> '{}'\" }}",
        out.display()
    );
    let mut keys = configured(&text).unwrap();
    assert_eq!(run(&mut keys, &["fire", "note", "up"]).unwrap(), "note up: no action");
    assert!(keys.worker.idle());
    assert!(run(&mut keys, &["fire", "note", "down"]).unwrap().starts_with("note down: queued shell echo"));
    assert_eq!(read_when_written(&out), "note down\n");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn started_keys_install_the_rules_and_run_chords() {
    // No [keys] table: Started installs nothing.
    calls();
    let mut off = configured("").unwrap();
    test_cx("", |cx| off.on_event(Event::Started, cx));
    drop(off);
    assert!(calls().is_empty());

    let (l, port) = listener();
    let text = format!("[keys]\nhyper = \"caps_lock\"\n{PTT}").replace("PORT", &port.to_string());
    let mut keys = configured(&text).unwrap();
    assert!(keys.remap && calls().is_empty());
    let send = |keys: &mut Keys, event| test_cx("", |cx| keys.on_event(event, cx));
    assert!(!send(&mut keys, Event::Started));
    assert_eq!(calls(), ["start", "set_remap"]);
    send(&mut keys, Event::Chord { index: 0, down: true });
    send(&mut keys, Event::Wake);
    send(&mut keys, Event::Chord { index: 7, down: false });
    send(&mut keys, Event::Chord { index: 0, down: false });
    let heads = serve(&l, 2, "200 OK");
    let lines: Vec<_> = heads.iter().map(|h| h.lines().next().unwrap()).collect();
    assert_eq!(lines, ["POST /pipeline/listen/start HTTP/1.1", "POST /pipeline/listen/stop HTTP/1.1"]);

    // A reload to F18 keeps the tap and clears the remap; turning keys off stops the tap.
    let table = |text: &str| parse(text).unwrap().section("keys").unwrap().unwrap();
    keys.configure(&table("[keys]\nhyper = \"F18\"")).unwrap();
    assert_eq!(calls(), ["clear_remap"]);
    keys.configure(&table("")).unwrap();
    assert_eq!(calls(), ["stop", "clear_remap"]);
    assert_eq!(keys.status(), "keys: off");
    drop(keys);
    // The stub's clear fails, so dropping the module tries once more.
    assert_eq!(calls(), ["clear_remap"]);
}

#[test]
fn started_logs_conflicts_with_hotkeys_bound_after_it() {
    calls();
    let mut keys = configured("[keys]\nhyper = \"F18\"").unwrap();
    test_cx("", |cx| keys.on_event(Event::Started, cx));
    assert_eq!(calls(), ["start"]);
    // The controller binds the global hotkeys after `Started`; the check runs after that.
    HOTKEYS.with(|h| *h.borrow_mut() = vec!["F18".into()]);
    run_later();
    assert_eq!(calls(), ["log conflict: a global hotkey uses the hyper key, which the tap swallows"]);
    HOTKEYS.with(|h| h.borrow_mut().clear());
    drop(keys);
    calls();
}

const DICTATE: &str = r#"
    [[keys.chord]]
    name = "dictate"
    keys = ["right_cmd", "right_shift"]
    on_down = { flick = "dictation start" }
    on_up = { flick = "dictation stop" }
"#;

#[test]
fn flick_actions_go_to_the_local_hook_not_the_worker() {
    sent();
    let mut keys = configured(DICTATE).unwrap();
    assert_eq!(
        keys.list(),
        "0\tdictate\tright_cmd+right_shift\tdown: flick dictation start\tup: flick dictation stop"
    );
    let send = |keys: &mut Keys, event| test_cx("", |cx| keys.on_event(event, cx));
    send(&mut keys, Event::Chord { index: 0, down: true });
    send(&mut keys, Event::Chord { index: 0, down: false });
    assert_eq!(run(&mut keys, &["fire", "dictate", "down"]).unwrap(), "dictate down: queued flick dictation start");
    assert_eq!(sent(), ["dictation start", "dictation stop", "dictation start"]);
    assert!(keys.worker.idle());
    let err = configured("[[keys.chord]]\nname = \"x\"\nkeys = [\"fn\"]\non_up = { flick = \"a 'b\" }");
    assert_eq!(err.err().unwrap(), "[keys] chord \"x\": flick \"a 'b\": unclosed '");
}

#[test]
fn the_example_config_lists_every_key() {
    crate::config::example::assert_documents::<Settings>("keys");
    crate::config::example::assert_documents::<ChordSpec>("keys.chord");
}
