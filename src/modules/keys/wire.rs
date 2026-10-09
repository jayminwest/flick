//! What runs the `keys` rules: the shared key tap (`platform::keytap`) driving a
//! `core::keys::Engine`, and the Caps Lock to F18 HID remap (`platform::hid`).
//!
//! Nothing is installed until `Wire::start` (on `Event::Started`); a `Keys` that is only
//! configured, as in a reload's throwaway registry or the tests, never touches macOS. Empty
//! rules mean no tap and no remap. The tap thread runs the engine and posts chord edges as
//! `Event::Chord`; the module runs their actions on the main thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError};
use std::time::Instant;

use crate::core::Event;
use crate::core::keys::names::keycode;
use crate::core::keys::{Engine, Output, Rules, TapEvent, TapKind, Verdict};
use crate::platform::keytap::{self, Status};
use crate::platform::{app, events, hid, hotkeys, workspace};

/// Bundle ids of the apps the `keys` module replaces.
const HYPERKEY: &str = "com.knollsoft.Hyperkey";
const HAMMERSPOON: &str = "org.hammerspoon.Hammerspoon";

/// The macOS calls `Wire` makes: `REAL` in the app, stubs in tests.
pub struct Sys {
    pub start: fn(keytap::Handler, keytap::Rearm),
    pub stop: fn(),
    pub status: fn() -> Status,
    pub secure_input: fn() -> bool,
    pub current_flags: fn() -> u64,
    pub post_key: fn(u16),
    pub post: fn(Event),
    pub set_remap: fn() -> Result<(), String>,
    pub clear_remap: fn() -> Result<(), String>,
    pub remap_is_set: fn() -> Result<bool, String>,
    pub app_running: fn(&str) -> bool,
    pub hotkeys: fn() -> Vec<String>,
}

pub static REAL: Sys = Sys {
    start: keytap::start,
    stop: keytap::stop,
    status: keytap::status,
    secure_input: keytap::secure_input,
    current_flags: keytap::current_flags,
    post_key: keytap::post_key,
    post: events::post,
    set_remap,
    clear_remap,
    remap_is_set: hid::caps_to_f18_is_set,
    app_running: workspace::is_running,
    hotkeys: hotkeys::registered_keys,
};

/// Caps Lock sends F18 because Flick set it, so quitting must clear it.
static REMAPPED: AtomicBool = AtomicBool::new(false);

/// Set the remap; the first time, also hook quit (menu, `quit`, SIGTERM) to clear it again.
fn set_remap() -> Result<(), String> {
    static HOOKED: Once = Once::new();
    HOOKED.call_once(|| {
        app::on_terminate(clear_on_quit);
        app::quit_on_sigterm();
    });
    hid::set_caps_to_f18()?;
    REMAPPED.store(true, Ordering::SeqCst);
    Ok(())
}

fn clear_remap() -> Result<(), String> {
    hid::clear_caps_to_f18()?;
    REMAPPED.store(false, Ordering::SeqCst);
    Ok(())
}

fn clear_on_quit() {
    if REMAPPED.load(Ordering::SeqCst)
        && let Err(e) = clear_remap()
    {
        eprintln!("flick: keys: caps lock remap left set: {e}");
    }
}

fn lock(engine: &Mutex<Engine>) -> MutexGuard<'_, Engine> {
    engine.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Post chord edges for the main thread (`Keys::on_event` runs their actions).
fn post_edges(sys: &Sys, edges: &[(u16, bool)]) {
    for &(index, down) in edges {
        (sys.post)(Event::Chord { index, down });
    }
}

fn to_engine(ev: keytap::TapEvent) -> TapEvent {
    let kind = match ev.kind {
        keytap::Kind::Down => TapKind::Down,
        keytap::Kind::Up => TapKind::Up,
        keytap::Kind::FlagsChanged => TapKind::FlagsChanged,
    };
    let keytap::TapEvent { keycode, flags, autorepeat, injected, .. } = ev;
    TapEvent { kind, keycode, flags, autorepeat, injected }
}

/// Carry out the engine's answer on the tap thread: post the injected key and the edges,
/// and tell the tap what to do with the event.
fn act(sys: &Sys, out: &Output) -> keytap::Verdict {
    if let Some(key) = out.inject {
        (sys.post_key)(key);
    }
    post_edges(sys, &out.transitions);
    match out.verdict {
        Verdict::Pass => keytap::Verdict::Pass,
        Verdict::Drop => keytap::Verdict::Drop,
        Verdict::SetFlags(flags) => keytap::Verdict::SetFlags(flags),
    }
}

/// The tap handler: one engine step per event, the lock held only for that step.
fn handler(engine: Arc<Mutex<Engine>>, sys: &'static Sys) -> keytap::Handler {
    let t0 = Instant::now();
    Box::new(move |ev| {
        let now = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
        let out = lock(&engine).process(to_engine(ev), now);
        act(sys, &out)
    })
}

/// After a re-arm the tap may have missed key ups: release chords no longer held.
fn rearm(engine: Arc<Mutex<Engine>>, sys: &'static Sys) -> keytap::Rearm {
    Box::new(move || {
        let ups = lock(&engine).resync((sys.current_flags)());
        post_edges(sys, &ups);
    })
}

/// The installed state of the tap and the remap.
pub struct Wire {
    sys: &'static Sys,
    /// From `start` on, `apply` installs rules.
    started: bool,
    engine: Arc<Mutex<Engine>>,
    tap: bool,
    remapped: bool,
}

impl Default for Wire {
    fn default() -> Self {
        Wire::with(&REAL)
    }
}

impl Wire {
    pub fn with(sys: &'static Sys) -> Self {
        let engine = Arc::new(Mutex::new(Engine::new(Rules::default())));
        Wire { sys, started: false, engine, tap: false, remapped: false }
    }

    /// Install `rules` (and the remap if `remap`) from now on.
    pub fn start(&mut self, rules: &Rules, remap: bool) -> Vec<(u16, bool)> {
        self.started = true;
        self.apply(rules, remap)
    }

    /// Install `rules`: start, update or stop the tap, and set or clear the Caps Lock remap.
    /// Returns `up` for each chord held under the old rules (old indices). Before `start`,
    /// does nothing.
    pub fn apply(&mut self, rules: &Rules, remap: bool) -> Vec<(u16, bool)> {
        if !self.started {
            return vec![];
        }
        let ups = lock(&self.engine).set_rules(rules.clone());
        if rules.is_empty() && self.tap {
            (self.sys.stop)();
            self.tap = false;
        } else if !rules.is_empty() && !self.tap {
            let (engine, sys) = (Arc::clone(&self.engine), self.sys);
            (sys.start)(handler(Arc::clone(&engine), sys), rearm(engine, sys));
            self.tap = true;
        }
        if remap != self.remapped {
            let done = if remap { (self.sys.set_remap)() } else { (self.sys.clear_remap)() };
            match done {
                Ok(()) => self.remapped = remap,
                Err(e) => eprintln!("flick: keys: caps lock remap: {e}"),
            }
        }
        ups
    }

    /// After sleep the tap may have missed key ups: post `up` for chords no longer held.
    pub fn wake(&self) {
        if self.tap {
            let ups = lock(&self.engine).resync((self.sys.current_flags)());
            post_edges(self.sys, &ups);
        }
    }

    /// The tap's state in a few words, for the root item.
    pub fn tap_state(&self) -> &'static str {
        match (self.sys.status)() {
            Status::Off => "key tap not running",
            Status::Running => "key tap running",
            Status::NoPermission => "Accessibility needed",
            Status::Disabled => "key tap disabled by macOS",
        }
    }

    pub fn secure_input(&self) -> bool {
        (self.sys.secure_input)()
    }

    /// `set`, `not set`, or the error.
    pub fn remap_state(&self) -> String {
        match (self.sys.remap_is_set)() {
            Ok(true) => "set".into(),
            Ok(false) => "not set".into(),
            Err(e) => e,
        }
    }

    /// What may fight `rules`: Hyperkey remapping Caps Lock too, Hammerspoon's own taps,
    /// and global hotkeys on the hyper key or a chord's key. `names` names the chords.
    pub fn conflicts(&self, rules: &Rules, remap: bool, names: &[&str]) -> Vec<String> {
        let mut out = vec![];
        if remap && (self.sys.app_running)(HYPERKEY) {
            out.push("Hyperkey is running and also remaps Caps Lock; quit it".into());
        }
        if !rules.chords.is_empty() && (self.sys.app_running)(HAMMERSPOON) {
            out.push("Hammerspoon is running; its event taps may run the same chords".into());
        }
        let bound: Vec<u16> = (self.sys.hotkeys)().iter().filter_map(|k| keycode(k)).collect();
        if rules.hyper.is_some_and(|h| bound.contains(&h.source)) {
            out.push("a global hotkey uses the hyper key, which the tap swallows".into());
        }
        for (chord, name) in rules.chords.iter().zip(names) {
            if chord.key.is_some_and(|k| bound.contains(&k)) {
                out.push(format!("chord \"{name}\": its key is also a global hotkey"));
            }
        }
        out
    }
}

#[cfg(test)]
pub mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::core::keys::{Chord, F18, Hyper, flags};

    thread_local! {
        /// What the stub `Sys` was asked to do, in order.
        pub static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static HANDLER: RefCell<Option<(keytap::Handler, keytap::Rearm)>> =
            const { RefCell::new(None) };
        pub static FLAGS: RefCell<u64> = const { RefCell::new(0) };
        pub static SECURE: RefCell<bool> = const { RefCell::new(false) };
        pub static RUNNING: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
        pub static HOTKEYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
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
        set_remap: || {
            call("set_remap".into());
            Ok(())
        },
        clear_remap: || {
            call("clear_remap".into());
            Err("hidutil failed".into())
        },
        remap_is_set: || Ok(true),
        app_running: |id| RUNNING.with(|r| r.borrow().contains(&id)),
        hotkeys: || HOTKEYS.with(|h| h.borrow().clone()),
    };

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
        let hyper = Hyper { source: F18, flags: flags::CMD, tap: Some(53), tap_ms: 300 };
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

        // Held across a re-arm or sleep, the chord is released once.
        press(FlagsChanged, 61, BOTH);
        FLAGS.with(|f| *f.borrow_mut() = 0);
        rearm_now();
        wire.wake();
        assert_eq!(calls(), [chord(true), chord(false)]);

        // New rules keep the tap; a held chord is released (old index); empty rules stop it.
        press(FlagsChanged, 61, BOTH);
        calls();
        assert_eq!(wire.apply(&rules(), true), [(0, false)]);
        assert!(calls().is_empty());
        // Clearing fails in the stub, so it is retried on the next apply.
        wire.apply(&Rules::default(), false);
        assert_eq!(calls(), ["stop", "clear_remap"]);
        wire.apply(&Rules::default(), false);
        assert_eq!(calls(), ["clear_remap"]);
        wire.wake();
        assert!(calls().is_empty());
    }

    #[test]
    fn status_and_conflicts() {
        let wire = Wire::with(&STUB);
        assert_eq!(wire.tap_state(), "Accessibility needed");
        assert!(!wire.secure_input());
        assert_eq!(wire.remap_state(), "set");
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
}
