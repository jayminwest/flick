//! Module `keys`: key triggers. `[[keys.chord]]` entries run an action when a set of keys
//! goes down and when it comes up; `[keys] hyper` turns one key into cmd+ctrl+alt+shift.
//! Off unless configured: no chords and no hyper means no item and no worker thread.
//! The only id is `keys:status`, the Key Triggers root item.
//!
//! Configure checks key names with `core::keys::names` and compiles the engine's `Rules`;
//! from `Event::Started` on, `wire` installs them in the key tap (and the Caps Lock remap for
//! `hyper = "caps_lock"`), and each `Event::Chord` the tap posts runs that chord's action.

mod action;
mod wire;

use action::{Action, Job, Spec, Worker};
use serde::Deserialize;
use wire::Wire;

use crate::config::Section;
use crate::core::keys::names::{KeyName, keycode, parse_chord, parse_mods};
use crate::core::keys::{self, CAPS_LOCK, F18, Rules};
use crate::core::{Cx, Event, Icon, Item, ItemId, Module, Outcome, unknown_verb};

/// Table `[keys]`. `hyper` names the key that acts as Hyper (`caps_lock`, or any key name;
/// empty: off); while it is held, keys get `hyper_mods`; a press shorter than
/// `hyper_tap_ms` with no other key sends `hyper_tap` (empty: nothing).
#[derive(Deserialize)]
#[serde(default)]
struct Settings {
    hyper: String,
    hyper_mods: String,
    hyper_tap: String,
    hyper_tap_ms: u32,
    chord: Vec<ChordSpec>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hyper: String::new(),
            hyper_mods: "cmd+ctrl+alt+shift".into(),
            hyper_tap: "Escape".into(),
            hyper_tap_ms: 300,
            chord: vec![],
        }
    }
}

/// `[[keys.chord]]`: `name`, `keys` (all held at once), `on_down`, `on_up`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChordSpec {
    name: String,
    keys: Vec<String>,
    on_down: Option<Spec>,
    on_up: Option<Spec>,
}

/// A configured chord. Its index in `Keys::chords` is the chord index the key tap reports.
#[derive(Debug, PartialEq, Eq)]
struct Chord {
    name: String,
    keys: Vec<String>,
    rule: keys::Chord,
    on_down: Option<Action>,
    on_up: Option<Action>,
}

/// The hyper settings as written, for `list` and `status`.
#[derive(Debug, PartialEq, Eq)]
struct Hyper {
    key: String,
    mods: String,
    tap: Option<String>,
    tap_ms: u32,
}

#[derive(Default)]
pub struct Keys {
    hyper: Option<Hyper>,
    chords: Vec<Chord>,
    /// What the key tap matches: `hyper` and `chords` compiled, chords in the same order.
    rules: Rules,
    /// `hyper = "caps_lock"`: Caps Lock must send F18 (the HID remap).
    remap: bool,
    worker: Worker,
    wire: Wire,
}

/// The chords in `specs`, checked: names unique and non-empty, keys known (modifiers plus at
/// most one other key), actions valid.
fn chords(specs: Vec<ChordSpec>) -> Result<Vec<Chord>, String> {
    let mut chords: Vec<Chord> = vec![];
    for spec in specs {
        let name = spec.name;
        let bad = |why: String| format!("[keys] chord \"{name}\": {why}");
        if name.trim().is_empty() {
            return Err("[keys] chord: empty name".into());
        }
        if chords.iter().any(|c| c.name == name) {
            return Err(bad("duplicate name; keep one".into()));
        }
        let rule = parse_chord(&spec.keys).map_err(|e| bad(format!("keys: {e}")))?;
        let action = |spec: Option<Spec>| spec.map(Action::parse).transpose().map_err(bad);
        let (on_down, on_up) = (action(spec.on_down)?, action(spec.on_up)?);
        chords.push(Chord { name, keys: spec.keys, rule, on_down, on_up });
    }
    Ok(chords)
}

/// `[keys] hyper`, compiled: a non-modifier source key (`caps_lock` means F18, which the HID
/// remap turns Caps Lock into), `hyper_mods` flags and an optional `hyper_tap` key.
fn hyper(s: &Settings) -> Result<Option<(Hyper, keys::Hyper)>, String> {
    let key = s.hyper.trim();
    if key.is_empty() {
        return Ok(None);
    }
    let source = match key.parse::<KeyName>().map_err(|e| format!("[keys] hyper: {e}"))? {
        KeyName::Key(CAPS_LOCK) => F18,
        KeyName::Key(code) => code,
        KeyName::Mod(_) => {
            return Err(format!("[keys] hyper: `{key}` is a modifier; name a key such as caps_lock"));
        }
    };
    let flags = parse_mods(&s.hyper_mods).map_err(|e| format!("[keys] hyper_mods: {e}"))?;
    let tap_name = s.hyper_tap.trim();
    let tap = match tap_name {
        "" => None,
        name => Some(keycode(name).ok_or_else(|| format!("[keys] hyper_tap: unknown key `{name}`"))?),
    };
    let shown = Hyper {
        key: key.into(),
        mods: s.hyper_mods.trim().into(),
        tap: tap.map(|_| tap_name.into()),
        tap_ms: s.hyper_tap_ms,
    };
    Ok(Some((shown, keys::Hyper { source, flags, tap, tap_ms: s.hyper_tap_ms })))
}

impl Keys {
    fn off(&self) -> bool {
        self.rules.is_empty()
    }

    /// Queue chord `index`'s action for its `down` or up edge. `Ok` says what was queued.
    fn fire(&mut self, index: usize, down: bool) -> Result<String, String> {
        let chord = self.chords.get(index).ok_or_else(|| format!("keys: no chord {index}"))?;
        let state = if down { "down" } else { "up" };
        let action = if down { &chord.on_down } else { &chord.on_up };
        let Some(action) = action.clone() else {
            return Ok(format!("{} {state}: no action", chord.name));
        };
        let done = format!("{} {state}: queued {action}", chord.name);
        self.worker.send(Job { chord: chord.name.clone(), down, action })?;
        Ok(done)
    }

    /// One line per chord, `<index>\t<name>\t<keys>\tdown: <action>\tup: <action>`, then
    /// `hyper\t<key>\t<mods>\ttap: <key> <ms> ms` when hyper is set.
    fn list(&self) -> String {
        let shown = |a: &Option<Action>| a.as_ref().map_or("-".into(), ToString::to_string);
        let chords = self.chords.iter().enumerate().map(|(i, c)| {
            let keys = c.keys.join("+");
            format!("{i}\t{}\t{keys}\tdown: {}\tup: {}", c.name, shown(&c.on_down), shown(&c.on_up))
        });
        let hyper = self.hyper.iter().map(|h| {
            let tap = h.tap.as_deref().unwrap_or("-");
            format!("hyper\t{}\t{}\ttap: {tap} {} ms", h.key, h.mods, h.tap_ms)
        });
        chords.chain(hyper).collect::<Vec<_>>().join("\n")
    }

    /// One line for the root item and the status line.
    fn summary(&self) -> String {
        let n = self.chords.len();
        let mut parts = vec![format!("{n} chord{}", if n == 1 { "" } else { "s" })];
        if let Some(h) = &self.hyper {
            parts.push(format!("hyper on {}", h.key));
        }
        if self.hyper.is_some() && self.wire.secure_input() {
            parts.push("Secure input is on: hyper paused".into());
        } else {
            parts.push(self.wire.tap_state().into());
        }
        parts.join(" · ")
    }

    /// `flick keys status`: `off`, or the tap state and what is configured.
    fn status(&self) -> String {
        if self.off() {
            return "keys: off".into();
        }
        let on = |b: bool| if b { "on" } else { "off" };
        let mut lines = vec![
            format!("tap: {}", self.wire.tap_state()),
            format!("secure input: {}", on(self.wire.secure_input())),
            format!("chords: {}", self.chords.len()),
        ];
        if let Some(h) = &self.hyper {
            lines.push(format!("hyper: {} ({})", h.key, h.mods));
        }
        if self.remap {
            lines.push(format!("caps lock remap: {}", self.wire.remap_state()));
        }
        let names: Vec<&str> = self.chords.iter().map(|c| c.name.as_str()).collect();
        let conflicts = self.wire.conflicts(&self.rules, self.remap, &names);
        lines.extend(conflicts.into_iter().map(|c| format!("conflict: {c}")));
        lines.join("\n")
    }

    /// Queue each `(index, down)` edge, logging what fails.
    fn fire_all(&mut self, edges: &[(u16, bool)]) {
        for &(index, down) in edges {
            if let Err(e) = self.fire(usize::from(index), down) {
                eprintln!("flick: {e}");
            }
        }
    }

    /// Install the rules (from `Event::Started` on) and report conflicts to the log.
    fn start(&mut self) {
        let ups = self.wire.start(&self.rules, self.remap);
        self.fire_all(&ups);
        let names: Vec<&str> = self.chords.iter().map(|c| c.name.as_str()).collect();
        for c in self.wire.conflicts(&self.rules, self.remap, &names) {
            eprintln!("flick: keys: conflict: {c}");
        }
    }
}

impl Drop for Keys {
    /// Removed by a reload (`enabled = false`), or quit: stop the tap, clear the remap and
    /// end held chords.
    fn drop(&mut self) {
        let ups = self.wire.apply(&Rules::default(), false);
        self.fire_all(&ups);
    }
}

impl Module for Keys {
    fn id(&self) -> &'static str {
        "keys"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let mut s = table.get::<Settings>()?;
        let (hyper, rule) = hyper(&s)?.unzip();
        let chords = chords(std::mem::take(&mut s.chord))?;
        let rules = Rules { hyper: rule, chords: chords.iter().map(|c| c.rule).collect() };
        let remap = rule.is_some() && s.hyper.trim().parse() == Ok(KeyName::Key(CAPS_LOCK));
        // Once started, a reload installs the new rules; chords held now end under their
        // old index, before `chords` changes.
        let ups = self.wire.apply(&rules, remap);
        self.fire_all(&ups);
        (self.hyper, self.chords, self.rules, self.remap) = (hyper, chords, rules, remap);
        Ok(())
    }

    /// `Started` installs the rules; `Chord` runs an action; `Wake` releases chords whose
    /// key up was lost in sleep.
    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        match event {
            Event::Started => self.start(),
            Event::Chord { index, down } => self.fire_all(&[(index, down)]),
            Event::Wake => self.wire.wake(),
            _ => {}
        }
        false
    }

    /// The Key Triggers item, only when something is configured.
    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        if self.off() {
            return vec![];
        }
        let id = ItemId::new("keys", "status");
        vec![Item {
            subtitle: self.summary(),
            accessory: "Keys".into(),
            keywords: vec!["hyper chord trigger hotkey".into()],
            ..Item::new(id, "Key Triggers", "Show Status", Icon::Symbol("keyboard"))
        }]
    }

    fn activate(&mut self, id: &ItemId, _cx: &mut Cx) -> Outcome {
        Outcome::Stay((id.key() == "status").then(|| self.summary()))
    }

    fn verbs(&self) -> &'static str {
        "keys list | keys status | keys fire <chord> down|up"
    }

    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        match args {
            [verb] if verb == "list" => Ok(self.list()),
            [verb] if verb == "status" => Ok(self.status()),
            [verb, name, state] if verb == "fire" => {
                let index = self.chords.iter().position(|c| c.name == *name);
                let index = index.ok_or_else(|| format!("keys: no chord \"{name}\""))?;
                match state.as_str() {
                    "down" => self.fire(index, true),
                    "up" => self.fire(index, false),
                    _ => Err(format!("keys: fire {name}: expected down or up, got \"{state}\"")),
                }
            }
            _ => Err(unknown_verb("keys", args)),
        }
    }
}

#[cfg(test)]
mod tests;
