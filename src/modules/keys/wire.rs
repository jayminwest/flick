//! What runs the `keys` rules: the shared key tap (`platform::keytap`) driving a
//! `core::keys::Engine`, the Caps Lock to Right Control HID remap (`platform::hid`), and the
//! hotkeys' control layer, on while the hyper key is held (`platform::hotkeys`).
//!
//! Nothing is installed until `Wire::start` (on `Event::Started`); a `Keys` that is only
//! configured, as in a reload's throwaway registry or the tests, never touches macOS. Empty
//! rules mean no tap and no remap. The tap thread runs the engine and posts chord edges as
//! `Event::Chord`; the module runs their actions on the main thread.
//!
//! Secure input hides key down/up from the tap but not the hyper key's flag changes, so the
//! tap still turns the control layer on and off: Caps+H arrives as ctrl+H and the layer fires
//! the hyper+H hotkey (flick-1271). Under secure input a tap of the hyper key sends nothing,
//! since the tap cannot tell whether another key was pressed meanwhile.

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
    /// Turn the hotkeys' control layer on or off for these hyper flags (main thread).
    pub layer: fn(bool, u64),
    pub set_remap: fn() -> Result<(), String>,
    pub clear_remap: fn() -> Result<(), String>,
    pub remap_is_set: fn() -> Result<bool, String>,
    pub app_running: fn(&str) -> bool,
    pub hotkeys: fn() -> Vec<String>,
    /// Run a job on the main queue after the current call returns.
    pub later: fn(Box<dyn FnOnce() + Send>),
    /// Log one line.
    pub log: fn(&str),
}

pub static REAL: Sys = Sys {
    start: keytap::start,
    stop: keytap::stop,
    status: keytap::status,
    secure_input: keytap::secure_input,
    current_flags: keytap::current_flags,
    post_key: keytap::post_key,
    post: events::post,
    layer: hotkeys::set_ctrl_layer,
    set_remap,
    clear_remap,
    remap_is_set: hid::caps_remap_is_set,
    app_running: workspace::is_running,
    hotkeys: hotkeys::registered_keys,
    later: |job| events::on_main(job),
    log: |line| eprintln!("flick: keys: {line}"),
};

/// Caps Lock sends Right Control because Flick set it, so quitting must clear it.
static REMAPPED: AtomicBool = AtomicBool::new(false);

/// Set the remap; the first time, also hook quit (menu, `quit`, SIGTERM) to clear it again.
fn set_remap() -> Result<(), String> {
    static HOOKED: Once = Once::new();
    HOOKED.call_once(|| {
        app::on_terminate(clear_on_quit);
        app::quit_on_sigterm();
    });
    hid::set_caps_remap()?;
    REMAPPED.store(true, Ordering::SeqCst);
    Ok(())
}

fn clear_remap() -> Result<(), String> {
    hid::clear_caps_remap()?;
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
/// switch the control layer for `hyper` flags, and tell the tap what to do with the event.
fn act(sys: &'static Sys, out: &Output, hyper: u64) -> keytap::Verdict {
    if let Some(key) = out.inject.filter(|_| !(sys.secure_input)()) {
        (sys.post_key)(key);
    }
    post_edges(sys, &out.transitions);
    if let Some(on) = out.hyper {
        (sys.later)(Box::new(move || (sys.layer)(on, hyper)));
    }
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
        let (out, hyper) = {
            let mut engine = lock(&engine);
            (engine.process(to_engine(ev), now), engine.hyper_flags())
        };
        act(sys, &out, hyper)
    })
}

/// After a re-arm the tap may have missed key ups: release chords no longer held, and turn
/// the control layer off (the engine forgot the hyper hold).
fn rearm(engine: Arc<Mutex<Engine>>, sys: &'static Sys) -> keytap::Rearm {
    Box::new(move || {
        let ups = lock(&engine).resync((sys.current_flags)());
        post_edges(sys, &ups);
        (sys.later)(Box::new(|| (sys.layer)(false, 0)));
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

    /// Install `rules` (and the remap if `remap`) from now on. Without `remap`, also clear
    /// Flick's Caps Lock entry left set by an earlier run that crashed: nothing else would.
    pub fn start(&mut self, rules: &Rules, remap: bool) -> Vec<(u16, bool)> {
        self.started = true;
        if !remap {
            match (self.sys.remap_is_set)() {
                Ok(set) => self.remapped = set,
                Err(e) => eprintln!("flick: keys: caps lock remap: {e}"),
            }
        }
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
        if self.tap {
            // The engine forgot the hyper hold: a layer it turned on must not stay on.
            (self.sys.layer)(false, 0);
        }
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

    /// After sleep the tap may have missed key ups: post `up` for chords no longer held, and
    /// turn the control layer off.
    pub fn wake(&self) {
        if self.tap {
            let ups = lock(&self.engine).resync((self.sys.current_flags)());
            post_edges(self.sys, &ups);
            (self.sys.layer)(false, 0);
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
        conflicts(self.sys, rules, remap, names)
    }

    /// Log `conflicts` on the main queue's next turn, not now: `Event::Started` runs before
    /// the controller binds the global hotkeys, so a check now would miss them (flick-6968).
    pub fn log_conflicts_later(&self, rules: &Rules, remap: bool, names: Vec<String>) {
        let (sys, rules) = (self.sys, rules.clone());
        (sys.later)(Box::new(move || {
            for c in conflicts(sys, &rules, remap, &names) {
                (sys.log)(&format!("conflict: {c}"));
            }
        }));
    }
}

/// `Wire::conflicts`, with the macOS calls of `sys`.
fn conflicts<S: AsRef<str>>(sys: &Sys, rules: &Rules, remap: bool, names: &[S]) -> Vec<String> {
    let mut out = vec![];
    if remap && (sys.app_running)(HYPERKEY) {
        out.push("Hyperkey is running and also remaps Caps Lock; quit it".into());
    }
    if !rules.chords.is_empty() && (sys.app_running)(HAMMERSPOON) {
        out.push("Hammerspoon is running; its event taps may run the same chords".into());
    }
    let bound: Vec<u16> = (sys.hotkeys)().iter().filter_map(|k| keycode(k)).collect();
    if rules.hyper.is_some_and(|h| bound.contains(&h.source)) {
        out.push("a global hotkey uses the hyper key, which the tap swallows".into());
    }
    for (chord, name) in rules.chords.iter().zip(names) {
        if chord.key.is_some_and(|k| bound.contains(&k)) {
            out.push(format!("chord \"{}\": its key is also a global hotkey", name.as_ref()));
        }
    }
    out
}

#[cfg(test)]
pub mod tests;
