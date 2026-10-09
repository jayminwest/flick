//! The key engine: a pure state machine over the keyboard events a session event tap sees.
//! It turns modifier chords into `(index, down)` transitions and a hyper key into extra
//! flags on every key pressed while it is held, plus a key sent when it is only tapped.
//! The platform tap feeds it `TapEvent`s and applies its `Output`; nothing here touches
//! macOS, so the whole machine unit-tests.

pub mod names;

/// `CGEventFlags` bits. The generic bits are set while either side is held; the device
/// bits (`NX_DEVICE*KEYMASK`) tell left from right.
pub mod flags {
    pub const SHIFT: u64 = 0x2_0000;
    pub const CTRL: u64 = 0x4_0000;
    pub const ALT: u64 = 0x8_0000;
    pub const CMD: u64 = 0x10_0000;
    pub const FN: u64 = 0x80_0000;
    pub const LEFT_CTRL: u64 = 0x1;
    pub const LEFT_SHIFT: u64 = 0x2;
    pub const RIGHT_SHIFT: u64 = 0x4;
    pub const LEFT_CMD: u64 = 0x8;
    pub const RIGHT_CMD: u64 = 0x10;
    pub const LEFT_ALT: u64 = 0x20;
    pub const RIGHT_ALT: u64 = 0x40;
    pub const RIGHT_CTRL: u64 = 0x2000;
}

/// cmd|ctrl|alt|shift, the flags a hyper key adds (Hyperkey's `hyperFlags`).
pub const HYPER_FLAGS: u64 = flags::CMD | flags::CTRL | flags::ALT | flags::SHIFT;
/// Virtual keycode of Caps Lock.
pub const CAPS_LOCK: u16 = 57;
/// Virtual keycode of F18, what the HID remap turns Caps Lock into.
pub const F18: u16 = 79;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapKind {
    Down,
    Up,
    FlagsChanged,
}

/// One keyboard event as the tap sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapEvent {
    pub kind: TapKind,
    pub keycode: u16,
    /// The event's `CGEventFlags`.
    pub flags: u64,
    /// `kCGKeyboardEventAutorepeat`.
    pub autorepeat: bool,
    /// Posted by Flick itself (e.g. the hyper tap key): passed through untouched.
    pub injected: bool,
}

/// A chord: active while every bit of `mods` is set and `key`, if any, is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub mods: u64,
    pub key: Option<u16>,
}

/// A hyper key: `source` adds `flags` to every event while held; a tap of it shorter than
/// `tap_ms`, with no other key pressed meanwhile, sends `tap` instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hyper {
    /// A non-modifier key (it arrives as key down/up, not as a flags change).
    pub source: u16,
    pub flags: u64,
    pub tap: Option<u16>,
    pub tap_ms: u32,
}

/// Everything the engine acts on. Chord transitions carry the index into `chords`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pub hyper: Option<Hyper>,
    pub chords: Vec<Chord>,
}

impl Rules {
    /// No hyper and no chords: no tap is needed.
    pub fn is_empty(&self) -> bool {
        self.hyper.is_none() && self.chords.is_empty()
    }
}

/// What the tap does with the event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Drop,
    /// Pass with its flags replaced by these.
    SetFlags(u64),
}

/// The engine's answer to one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub verdict: Verdict,
    /// A key to post (down and up), marked injected.
    pub inject: Option<u16>,
    /// `(chord index, down)` edges, in order.
    pub transitions: Vec<(u16, bool)>,
}

/// One bit per virtual keycode below 128 (all keyboard keycodes are).
#[derive(Clone, Copy, Debug, Default)]
struct KeySet(u128);

impl KeySet {
    fn has(self, code: u16) -> bool {
        code < 128 && self.0 & (1 << code) != 0
    }

    fn set(&mut self, code: u16, on: bool) {
        if code < 128 {
            if on { self.0 |= 1 << code } else { self.0 &= !(1 << code) }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Held {
    since_ms: u64,
    other_key: bool,
}

#[derive(Debug, Default)]
pub struct Engine {
    rules: Rules,
    flags: u64,
    down: KeySet,
    /// Chord keys whose down was dropped, so their up is dropped too.
    swallowed: KeySet,
    active: Vec<bool>,
    hyper: Option<Held>,
}

impl Engine {
    pub fn new(rules: Rules) -> Self {
        let mut engine = Engine::default();
        engine.set_rules(rules);
        engine
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    /// Swap the rules; returns `up` for every chord that was active, and forgets held keys.
    pub fn set_rules(&mut self, rules: Rules) -> Vec<(u16, bool)> {
        let ups = self.release_all();
        self.active = vec![false; rules.chords.len()];
        self.rules = rules;
        self.hyper = None;
        self.swallowed = KeySet::default();
        ups
    }

    pub fn process(&mut self, ev: TapEvent, now_ms: u64) -> Output {
        let mut out = Output { verdict: Verdict::Pass, inject: None, transitions: Vec::new() };
        if ev.injected {
            return out;
        }
        self.flags = ev.flags;
        let key = ev.kind != TapKind::FlagsChanged;
        if let Some(hyper) = self.rules.hyper.filter(|h| key && h.source == ev.keycode) {
            out.verdict = Verdict::Drop;
            match (ev.kind, self.hyper) {
                (TapKind::Down, None) => {
                    self.hyper = Some(Held { since_ms: now_ms, other_key: false });
                }
                (TapKind::Up, Some(held)) => {
                    self.hyper = None;
                    let quick = now_ms.saturating_sub(held.since_ms) < u64::from(hyper.tap_ms);
                    out.inject = hyper.tap.filter(|_| quick && !held.other_key);
                }
                _ => {}
            }
            return out;
        }
        if key && !ev.autorepeat {
            self.down.set(ev.keycode, ev.kind == TapKind::Down);
            if let (TapKind::Down, Some(held)) = (ev.kind, &mut self.hyper) {
                held.other_key = true;
            }
        }
        out.transitions = self.update();
        if key && self.is_chord_key(ev.keycode) {
            let swallow = match ev.kind {
                TapKind::Up => self.swallowed.has(ev.keycode),
                _ => self.chord_on_key(ev.keycode),
            };
            self.swallowed.set(ev.keycode, swallow && ev.kind == TapKind::Down);
            if swallow {
                out.verdict = Verdict::Drop;
                return out;
            }
        }
        if let (Some(hyper), Some(_)) = (self.rules.hyper, self.hyper) {
            out.verdict = Verdict::SetFlags(ev.flags | hyper.flags);
        }
        out
    }

    /// Re-read state after the tap missed events (re-arm, wake): adopt `current_flags`,
    /// forget held keys and the hyper hold, and return `up` for every chord no longer held.
    pub fn resync(&mut self, current_flags: u64) -> Vec<(u16, bool)> {
        self.flags = current_flags;
        self.down = KeySet::default();
        self.swallowed = KeySet::default();
        self.hyper = None;
        self.update_only(false)
    }

    fn release_all(&mut self) -> Vec<(u16, bool)> {
        self.down = KeySet::default();
        self.flags = 0;
        self.update()
    }

    fn holds(&self, chord: Chord) -> bool {
        (chord.mods != 0 || chord.key.is_some())
            && self.flags & chord.mods == chord.mods
            && chord.key.is_none_or(|k| self.down.has(k))
    }

    /// Recompute each chord, returning the edges.
    fn update(&mut self) -> Vec<(u16, bool)> {
        self.update_only(true)
    }

    /// Like `update`, but with `downs` false only ends chords.
    fn update_only(&mut self, downs: bool) -> Vec<(u16, bool)> {
        let mut edges = Vec::new();
        for (i, chord) in self.rules.chords.iter().enumerate() {
            let now = self.holds(*chord);
            if self.active[i] != now && (downs || !now) {
                self.active[i] = now;
                edges.push((i as u16, now));
            }
        }
        edges
    }

    fn is_chord_key(&self, code: u16) -> bool {
        self.rules.chords.iter().any(|c| c.key == Some(code))
    }

    fn chord_on_key(&self, code: u16) -> bool {
        self.rules.chords.iter().zip(&self.active).any(|(c, &on)| on && c.key == Some(code))
    }
}

#[cfg(test)]
mod tests {
    use super::flags::*;
    use super::*;

    const H: u16 = 4;
    const ESC: u16 = 53;
    const RCMD: u64 = CMD | RIGHT_CMD;
    const RALT: u64 = ALT | RIGHT_ALT;

    fn ev(kind: TapKind, keycode: u16, flags: u64) -> TapEvent {
        TapEvent { kind, keycode, flags, autorepeat: false, injected: false }
    }
    fn flags_ev(flags: u64) -> TapEvent {
        ev(TapKind::FlagsChanged, 54, flags)
    }
    fn down(code: u16) -> TapEvent {
        ev(TapKind::Down, code, 0)
    }
    fn up(code: u16) -> TapEvent {
        ev(TapKind::Up, code, 0)
    }
    fn repeat(mut e: TapEvent) -> TapEvent {
        e.autorepeat = true;
        e
    }

    fn ptt() -> Engine {
        Engine::new(Rules { hyper: None, chords: vec![Chord { mods: RCMD | RALT, key: None }] })
    }

    fn hyper(tap: Option<u16>) -> Engine {
        let hyper = Hyper { source: F18, flags: HYPER_FLAGS, tap, tap_ms: 300 };
        Engine::new(Rules { hyper: Some(hyper), chords: Vec::new() })
    }

    #[test]
    fn right_cmd_right_alt_chord_fires_down_once_and_up_once() {
        let mut e = ptt();
        assert_eq!(e.process(flags_ev(RCMD), 0).transitions, []);
        let out = e.process(flags_ev(RCMD | RALT), 1);
        assert_eq!((out.verdict, out.transitions), (Verdict::Pass, vec![(0, true)]));
        assert_eq!(e.process(flags_ev(RCMD | RALT | SHIFT), 2).transitions, []);
        assert_eq!(e.process(ev(TapKind::Down, H, RCMD | RALT), 3).transitions, []);
        assert_eq!(e.process(flags_ev(RALT), 4).transitions, [(0, false)]);
        assert_eq!(e.process(flags_ev(0), 5).transitions, []);
    }

    #[test]
    fn left_modifiers_never_fire_a_right_chord() {
        let mut e = ptt();
        let left = CMD | LEFT_CMD | ALT | LEFT_ALT;
        assert_eq!(e.process(flags_ev(left), 0).transitions, []);
        assert_eq!(e.process(flags_ev(left | RIGHT_CMD), 0).transitions, []);
    }

    #[test]
    fn chord_key_is_swallowed_and_repeats_never_fire() {
        let chord = Chord { mods: CTRL, key: Some(H) };
        let mut e = Engine::new(Rules { hyper: None, chords: vec![chord] });
        // Without the modifier the key types normally.
        assert_eq!(e.process(down(H), 0).verdict, Verdict::Pass);
        assert_eq!(e.process(up(H), 0).verdict, Verdict::Pass);
        e.process(flags_ev(CTRL), 0);
        let out = e.process(ev(TapKind::Down, H, CTRL), 1);
        assert_eq!((out.verdict, out.transitions), (Verdict::Drop, vec![(0, true)]));
        let out = e.process(repeat(ev(TapKind::Down, H, CTRL)), 2);
        assert_eq!((out.verdict, out.transitions), (Verdict::Drop, vec![]));
        // The modifier goes first: the chord ends, but the key's up is still swallowed.
        assert_eq!(e.process(flags_ev(0), 3).transitions, [(0, false)]);
        let out = e.process(up(H), 4);
        assert_eq!((out.verdict, out.transitions), (Verdict::Drop, vec![]));
        // A repeat of a key that was never seen down changes nothing.
        e.process(flags_ev(CTRL), 5);
        assert_eq!(e.process(repeat(ev(TapKind::Down, H, CTRL)), 6).transitions, []);
    }

    #[test]
    fn hyper_sets_flags_on_keys_while_held() {
        let mut e = hyper(Some(ESC));
        assert_eq!(e.process(down(F18), 0).verdict, Verdict::Drop);
        assert_eq!(e.process(repeat(down(F18)), 50).verdict, Verdict::Drop);
        assert_eq!(
            e.process(ev(TapKind::Down, H, SHIFT), 60).verdict,
            Verdict::SetFlags(HYPER_FLAGS)
        );
        assert_eq!(e.process(up(H), 70).verdict, Verdict::SetFlags(HYPER_FLAGS));
        assert_eq!(e.process(flags_ev(CMD), 80).verdict, Verdict::SetFlags(HYPER_FLAGS));
        let out = e.process(up(F18), 100);
        assert_eq!((out.verdict, out.inject), (Verdict::Drop, None));
        assert_eq!(e.process(down(H), 110).verdict, Verdict::Pass);
    }

    #[test]
    fn hyper_tap_sends_the_tap_key_only_when_quick_and_alone() {
        let mut e = hyper(Some(ESC));
        e.process(down(F18), 1000);
        assert_eq!(e.process(up(F18), 1299).inject, Some(ESC));
        e.process(down(F18), 2000);
        assert_eq!(e.process(up(F18), 2300).inject, None);
        // An up without a down (tap started mid-hold) sends nothing.
        assert_eq!(e.process(up(F18), 2400).inject, None);
        let mut e = hyper(None);
        e.process(down(F18), 0);
        assert_eq!(e.process(up(F18), 10).inject, None);
    }

    #[test]
    fn injected_events_pass_untouched() {
        let mut e = hyper(Some(ESC));
        e.process(down(F18), 0);
        let mut esc = down(ESC);
        esc.injected = true;
        let out = e.process(esc, 1);
        assert_eq!((out.verdict, out.inject, out.transitions), (Verdict::Pass, None, vec![]));
        assert_eq!(e.process(up(F18), 2).inject, Some(ESC));
    }

    #[test]
    fn resync_releases_held_chords_and_hyper() {
        let mut e = ptt();
        e.process(flags_ev(RCMD | RALT), 0);
        assert_eq!(e.resync(RCMD | RALT), []);
        assert_eq!(e.resync(0), [(0, false)]);
        // Resync never fires a down: the next event does.
        assert_eq!(e.resync(RCMD | RALT), []);
        assert_eq!(e.process(flags_ev(RCMD | RALT), 1).transitions, [(0, true)]);

        let mut e = hyper(Some(ESC));
        e.process(down(F18), 0);
        e.resync(0);
        assert_eq!(e.process(down(H), 1).verdict, Verdict::Pass);
    }

    #[test]
    fn set_rules_releases_active_chords() {
        let mut e = ptt();
        assert!(!e.rules().is_empty());
        e.process(flags_ev(RCMD | RALT), 0);
        assert_eq!(e.set_rules(Rules::default()), [(0, false)]);
        assert!(e.rules().is_empty());
        assert_eq!(e.process(flags_ev(RCMD | RALT), 1).transitions, []);
    }

    #[test]
    fn empty_chords_never_fire_and_high_keycodes_are_ignored() {
        let chords = vec![Chord { mods: 0, key: None }, Chord { mods: 0, key: Some(200) }];
        let mut e = Engine::new(Rules { hyper: None, chords });
        assert_eq!(e.process(down(200), 0).transitions, []);
        assert_eq!(e.process(flags_ev(CMD), 0).transitions, []);
    }
}
