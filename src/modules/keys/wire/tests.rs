//! The stub `Sys` and the wire tests; the module tests use the stub too.

use std::cell::RefCell;

use super::*;
use crate::core::keys::{Chord, F18, Hyper, RIGHT_CTRL_KEY, flags};

thread_local! {
    /// What the stub `Sys` was asked to do, in order.
    pub static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static HANDLER: RefCell<Option<(keytap::Handler, keytap::Rearm)>> =
        const { RefCell::new(None) };
    pub static FLAGS: RefCell<u64> = const { RefCell::new(0) };
    pub static SECURE: RefCell<bool> = const { RefCell::new(false) };
    pub static RUNNING: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    pub static HOTKEYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Jobs the stub `later` queued, for `run_later`.
    static LATER: RefCell<Vec<Box<dyn FnOnce() + Send>>> = const { RefCell::new(Vec::new()) };
    /// What the stub `remap_is_set` answers.
    pub static REMAP_SET: RefCell<Result<bool, String>> = const { RefCell::new(Ok(false)) };
}

fn call(s: String) {
    CALLS.with(|c| c.borrow_mut().push(s));
}

pub fn calls() -> Vec<String> {
    CALLS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

pub static STUB: Sys = Sys {
    start: |h, r| {
        call("start".into());
        HANDLER.with(|s| *s.borrow_mut() = Some((h, r)));
    },
    stop: || call("stop".into()),
    status: || Status::NoPermission,
    secure_input: || SECURE.with(|s| *s.borrow()),
    current_flags: || FLAGS.with(|f| *f.borrow()),
    post_key: |k| call(format!("post_key {k}")),
    post: |e| call(serde_json::to_string(&e).unwrap_or_default()),
    layer: |on, hyper| call(format!("layer {on} {hyper:#x}")),
    set_remap: || {
        call("set_remap".into());
        Ok(())
    },
    clear_remap: || {
        call("clear_remap".into());
        Err("hidutil failed".into())
    },
    remap_is_set: || REMAP_SET.with(|r| r.borrow().clone()),
    app_running: |id| RUNNING.with(|r| r.borrow().contains(&id)),
    hotkeys: || HOTKEYS.with(|h| h.borrow().clone()),
    later: |job| LATER.with(|l| l.borrow_mut().push(job)),
    log: |line| call(format!("log {line}")),
};

/// Run the jobs the stub `later` queued, as the main queue would after the current call.
pub fn run_later() {
    for job in LATER.with(|l| std::mem::take(&mut *l.borrow_mut())) {
        job();
    }
}

/// Feed one event to the stub tap's handler.
pub fn press(kind: keytap::Kind, keycode: u16, flags: u64) -> keytap::Verdict {
    let ev = keytap::TapEvent { kind, keycode, flags, autorepeat: false, injected: false };
    HANDLER.with(|s| s.borrow_mut().as_mut().map(|(h, _)| h(ev))).unwrap_or(keytap::Verdict::Pass)
}

pub fn rearm_now() {
    HANDLER.with(|s| {
        if let Some((_, r)) = s.borrow_mut().as_mut() {
            r();
        }
    });
}

const RIGHT_CMD: u64 = flags::CMD | flags::RIGHT_CMD;
const BOTH: u64 = RIGHT_CMD | flags::ALT | flags::RIGHT_ALT;

fn rules() -> Rules {
    let hyper = Hyper { source: F18, held_bit: 0, flags: flags::CMD, tap: Some(53), tap_ms: 300 };
    Rules { hyper: Some(hyper), chords: vec![Chord { mods: BOTH, key: None }] }
}

#[test]
fn nothing_is_installed_before_start_or_for_empty_rules() {
    calls();
    let mut wire = Wire::with(&STUB);
    assert!(wire.apply(&rules(), true).is_empty());
    wire.wake();
    assert!(wire.start(&Rules::default(), false).is_empty());
    assert!(calls().is_empty());
    assert!(!Wire::default().started);
}

#[test]
fn the_tap_runs_the_engine_and_posts_chord_edges() {
    use keytap::Kind::{Down, FlagsChanged, Up};
    calls();
    let mut wire = Wire::with(&STUB);
    wire.start(&rules(), true);
    assert_eq!(calls(), ["start", "set_remap"]);
    assert_eq!(press(FlagsChanged, 54, RIGHT_CMD), keytap::Verdict::Pass);
    press(FlagsChanged, 61, BOTH);
    press(FlagsChanged, 61, RIGHT_CMD);
    let chord = |down| format!(r#"{{"event":"chord","index":0,"down":{down}}}"#);
    assert_eq!(calls(), [chord(true), chord(false)]);
    // Hyper: F18 is dropped, H gets the hyper flags, a quick tap injects Escape.
    assert_eq!(press(Down, F18, 0), keytap::Verdict::Drop);
    assert_eq!(press(Down, 4, 0), keytap::Verdict::SetFlags(flags::CMD));
    press(Up, 4, 0);
    assert_eq!(press(Up, F18, 0), keytap::Verdict::Drop);
    press(Down, F18, 0);
    press(Up, F18, 0);
    assert_eq!(calls(), ["post_key 53"]);
    // Each hold turns the control layer on and off, on the main queue.
    run_later();
    assert_eq!(calls(), ["layer true 0x100000", "layer false 0x100000"].repeat(2));

    // Held across a re-arm or sleep, the chord is released once.
    press(FlagsChanged, 61, BOTH);
    FLAGS.with(|f| *f.borrow_mut() = 0);
    rearm_now();
    run_later();
    wire.wake();
    assert_eq!(calls(), [chord(true), chord(false), "layer false 0x0".into(), "layer false 0x0".into()]);

    // New rules keep the tap; a held chord is released (old index); empty rules stop it.
    press(FlagsChanged, 61, BOTH);
    calls();
    assert_eq!(wire.apply(&rules(), true), [(0, false)]);
    assert_eq!(calls(), ["layer false 0x0"]);
    // Clearing fails in the stub, so it is retried on the next apply.
    wire.apply(&Rules::default(), false);
    assert_eq!(calls(), ["layer false 0x0", "stop", "clear_remap"]);
    wire.apply(&Rules::default(), false);
    assert_eq!(calls(), ["clear_remap"]);
    wire.wake();
    assert!(calls().is_empty());
}

#[test]
fn under_secure_input_caps_still_drives_the_layer_but_never_taps() {
    use keytap::Kind::FlagsChanged;
    calls();
    let mut wire = Wire::with(&STUB);
    let hyper = Hyper {
        source: RIGHT_CTRL_KEY,
        held_bit: flags::RIGHT_CTRL,
        flags: flags::CMD,
        tap: Some(53),
        tap_ms: 300,
    };
    wire.start(&Rules { hyper: Some(hyper), chords: vec![] }, true);
    calls();
    SECURE.with(|s| *s.borrow_mut() = true);
    let held = flags::CTRL | flags::RIGHT_CTRL;
    assert_eq!(press(FlagsChanged, RIGHT_CTRL_KEY, held), keytap::Verdict::Drop);
    assert_eq!(press(FlagsChanged, RIGHT_CTRL_KEY, 0), keytap::Verdict::Drop);
    run_later();
    SECURE.with(|s| *s.borrow_mut() = false);
    assert_eq!(calls(), ["layer true 0x100000", "layer false 0x100000"]);
}

#[test]
fn start_clears_a_remap_left_by_a_crash_unless_hyper_is_caps_lock() {
    calls();
    REMAP_SET.with(|r| *r.borrow_mut() = Ok(true));
    // Hyper is caps_lock: `set_remap` keeps the existing entry; nothing to clear.
    Wire::with(&STUB).start(&rules(), true);
    assert_eq!(calls(), ["start", "set_remap"]);
    // No caps_lock hyper (or no rules at all): the stale entry is cleared.
    Wire::with(&STUB).start(&Rules::default(), false);
    assert_eq!(calls(), ["clear_remap"]);
    let mut rules = rules();
    rules.hyper = None;
    Wire::with(&STUB).start(&rules, false);
    assert_eq!(calls(), ["start", "clear_remap"]);
    // A failing read is logged and leaves the mapping alone.
    REMAP_SET.with(|r| *r.borrow_mut() = Err("no hidutil".into()));
    Wire::with(&STUB).start(&Rules::default(), false);
    assert!(calls().is_empty());
    REMAP_SET.with(|r| *r.borrow_mut() = Ok(false));
    Wire::with(&STUB).start(&Rules::default(), false);
    assert!(calls().is_empty());
}

#[test]
fn status_and_conflicts() {
    let wire = Wire::with(&STUB);
    assert_eq!(wire.tap_state(), "Accessibility needed");
    assert!(!wire.secure_input());
    assert_eq!(wire.remap_state(), "not set");
    REMAP_SET.with(|r| *r.borrow_mut() = Ok(true));
    assert_eq!(wire.remap_state(), "set");
    REMAP_SET.with(|r| *r.borrow_mut() = Err("no hidutil".into()));
    assert_eq!(wire.remap_state(), "no hidutil");
    REMAP_SET.with(|r| *r.borrow_mut() = Ok(false));
    let mut rules = rules();
    rules.chords.push(Chord { mods: flags::FN, key: Some(4) });
    assert!(wire.conflicts(&rules, true, &["ptt", "h"]).is_empty());
    RUNNING.with(|r| *r.borrow_mut() = vec![HYPERKEY, HAMMERSPOON]);
    HOTKEYS.with(|h| *h.borrow_mut() = vec!["F18".into(), "KeyH".into(), "Nope".into()]);
    assert_eq!(
        wire.conflicts(&rules, true, &["ptt", "h"]),
        [
            "Hyperkey is running and also remaps Caps Lock; quit it",
            "Hammerspoon is running; its event taps may run the same chords",
            "a global hotkey uses the hyper key, which the tap swallows",
            "chord \"h\": its key is also a global hotkey",
        ]
    );
    assert!(wire.conflicts(&Rules::default(), false, &[]).is_empty());
    RUNNING.with(|r| r.borrow_mut().clear());
    HOTKEYS.with(|h| h.borrow_mut().clear());
}
