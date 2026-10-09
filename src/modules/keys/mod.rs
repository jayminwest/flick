//! Module `keys`: key triggers. `[[keys.chord]]` entries run an action when a set of keys
//! goes down and when it comes up; `[keys] hyper` turns one key into cmd+ctrl+alt+shift.
//! Off unless configured: no chords and no hyper means no item and no worker thread.
//! The only id is `keys:status`, the Key Triggers root item.
//!
//! Key names stay strings here; the key tap that matches them is wired in flick-df71.

mod action;

use action::{Action, Job, Spec, Worker};
use serde::Deserialize;

use crate::config::Section;
use crate::core::{Cx, Icon, Item, ItemId, Module, Outcome, unknown_verb};

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
    on_down: Option<Action>,
    on_up: Option<Action>,
}

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
    worker: Worker,
}

/// The chords in `specs`, checked: names unique and non-empty, keys non-empty, actions valid.
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
        if spec.keys.is_empty() || spec.keys.iter().any(|k| k.trim().is_empty()) {
            return Err(bad("keys: name at least one key, none empty".into()));
        }
        let action = |spec: Option<Spec>| spec.map(Action::parse).transpose().map_err(bad);
        let (on_down, on_up) = (action(spec.on_down)?, action(spec.on_up)?);
        chords.push(Chord { name, keys: spec.keys, on_down, on_up });
    }
    Ok(chords)
}

impl Keys {
    fn off(&self) -> bool {
        self.hyper.is_none() && self.chords.is_empty()
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
        // flick-df71 replaces this with the key tap's status.
        parts.push("key tap not running".into());
        parts.join(" · ")
    }

    /// `flick keys status`: `off`, or the tap state and what is configured.
    fn status(&self) -> String {
        if self.off() {
            return "keys: off".into();
        }
        let mut lines = vec!["tap: not running".to_string(), format!("chords: {}", self.chords.len())];
        if let Some(h) = &self.hyper {
            lines.push(format!("hyper: {} ({})", h.key, h.mods));
        }
        lines.join("\n")
    }
}

impl Module for Keys {
    fn id(&self) -> &'static str {
        "keys"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let s = table.get::<Settings>()?;
        let chords = chords(s.chord)?;
        let hyper = (!s.hyper.trim().is_empty()).then(|| Hyper {
            key: s.hyper.trim().into(),
            mods: s.hyper_mods,
            tap: (!s.hyper_tap.trim().is_empty()).then(|| s.hyper_tap.trim().into()),
            tap_ms: s.hyper_tap_ms,
        });
        if hyper.as_ref().is_some_and(|h| h.mods.trim().is_empty()) {
            return Err("[keys] hyper_mods: name at least one modifier".into());
        }
        (self.hyper, self.chords) = (hyper, chords);
        Ok(())
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
mod tests {
    use super::action::tests::{listener, read_when_written, serve, temp};
    use super::*;
    use crate::config::parse;
    use crate::core::test_cx;

    fn configured(text: &str) -> Result<Keys, String> {
        let mut keys = Keys::default();
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
            "tap: not running\nchords: 1\nhyper: caps_lock (cmd+ctrl+alt+shift)"
        );
        let items = test_cx("", |cx| keys.items(cx));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "keys:status");
        assert_eq!(items[0].subtitle, "1 chord · hyper on caps_lock · key tap not running");
        let out = test_cx("", |cx| keys.activate(&items[0].id, cx));
        assert!(matches!(out, Outcome::Stay(Some(s)) if s == items[0].subtitle));
        let other = test_cx("", |cx| keys.activate(&ItemId::new("keys", "x"), cx));
        assert!(matches!(other, Outcome::Stay(None)));

        let text = "[keys]\nhyper = \"F18\"\nhyper_tap = \"\"\nhyper_tap_ms = 200";
        let keys = configured(text).unwrap();
        let h = keys.hyper.as_ref().unwrap();
        assert_eq!((h.tap.as_deref(), h.tap_ms), (None, 200));
        assert_eq!(keys.list(), "hyper\tF18\tcmd+ctrl+alt+shift\ttap: - 200 ms");
        assert_eq!(keys.summary(), "0 chords · hyper on F18 · key tap not running");
    }

    #[test]
    fn bad_tables_name_the_problem() {
        let chord = |body: &str| format!("[[keys.chord]]\n{body}");
        let cases = [
            (chord("name = \"a\"\nkeys = [\"fn\"]\non_dwn = { shell = \"x\" }"), "unknown field"),
            (chord("name = \"a\"\nkeys = [\"fn\"]\non_up = { exec = \"x\" }"), "unknown variant"),
            (chord("name = \" \"\nkeys = [\"fn\"]"), "[keys] chord: empty name"),
            (chord("name = \"a\"\nkeys = []"), "[keys] chord \"a\": keys: name at least"),
            (chord("name = \"a\"\nkeys = [\"\"]"), "keys: name at least"),
            (
                chord("name = \"a\"\nkeys = [\"fn\"]\non_down = { http = \"https://x/\" }"),
                "[keys] chord \"a\": http \"https://x/\": only http://",
            ),
            (
                chord("name = \"a\"\nkeys = [\"fn\"]\n") + &chord("name = \"a\"\nkeys = [\"fn\"]"),
                "[keys] chord \"a\": duplicate name",
            ),
            ("[keys]\nhyper = \"F18\"\nhyper_mods = \"\"".into(), "hyper_mods: name at least"),
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
}
