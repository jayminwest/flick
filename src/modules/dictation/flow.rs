//! The verbs and the main-thread steps of a dictation.
//!
//! `start` (chord down) checks the microphone and the programs, starts the recorder and
//! shows the pill. `stop` (chord up) discards a hold under `min_hold_ms`, else hands the
//! recorder to a worker and remembers the frontmost app. `wake` (on `ModuleChanged`) takes a
//! queued Esc, a recorder that ended by itself and the worker's results, and inserts:
//! first it waits, polling with `Hooks::later` and never blocking, until the chord's
//! modifiers are up (at most `RELEASE_CAP`), then it refuses under secure input or when
//! another app is in front (the text stays in `last`), else pastes or types it.
//!
//! Microphone not determined: `start` asks (`Hooks::ask_mic`) and records nothing this time,
//! so the first clip is never the silence `rec` would capture while the prompt shows.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::recorder::{self, BYTES_PER_SEC, Recorder};
use super::session::{Effect, Input, State};
use super::settings::Insert;
use super::worker::{self, Done, Job, Outcome};
use super::{Dictation, Mic, Pill, Probe, clip, engine};

/// Shift, control, option and command (`CGEventFlags`).
pub const MODIFIERS: u64 = 0x1e_0000;
/// The longest insertion waits for the chord's modifiers to come up.
pub const RELEASE_CAP: Duration = Duration::from_millis(500);
/// Seconds between modifier checks while waiting.
pub const RELEASE_POLL: f64 = 0.02;

/// The running dictation's parts.
#[derive(Default)]
pub struct Live {
    recorder: Option<Recorder>,
    started: Option<Instant>,
    /// The engine's cancel flag, while transcribing.
    cancel: Option<Arc<AtomicBool>>,
    /// The frontmost app at chord-up: the text goes in only if it is still in front.
    target: Option<i32>,
    /// Text waiting for the modifiers to come up, and until when it waits.
    pending: Option<(String, Instant)>,
}

impl Dictation {
    pub(super) fn start(&mut self) -> Result<String, String> {
        if !self.on {
            return Err("dictation: off: set a key in [dictation] to turn it on".into());
        }
        let state = self.session.state();
        if state != State::Idle {
            return Err(format!("dictation: already {}", state.name()));
        }
        self.preflight()?;
        if !std::mem::replace(&mut self.armed, true) {
            (self.hooks.arm)();
        }
        let home = (self.hooks.home)();
        let argv = recorder::argv(&self.settings.recorder_path(&home).display().to_string());
        let max = self.settings.max_seconds as usize * BYTES_PER_SEC;
        let recorder = match Recorder::start(self.hooks.spawn, &argv, max, self.hooks.notify) {
            Ok(r) => r,
            Err(e) => return Err(self.fail(&format!("dictation: {e}"), "The recorder did not start")),
        };
        let _ = self.session.step(Input::Start);
        (self.hooks.pill)(Pill::Recording(recorder.level()));
        self.live = Live { recorder: Some(recorder), started: Some((self.hooks.now)()), ..Live::default() };
        self.problem = None;
        Ok("dictation: recording".into())
    }

    /// The microphone and every program and file the dictation needs.
    fn preflight(&mut self) -> Result<(), String> {
        match (self.hooks.mic)() {
            Mic::Denied | Mic::Restricted => {
                let why = "dictation: microphone denied (System Settings > Privacy & Security > Microphone)";
                return Err(self.fail(why, "Microphone denied"));
            }
            Mic::NotDetermined => {
                (self.hooks.ask_mic)();
                let why = "dictation: asked for microphone access; hold again once it is allowed";
                return Err(self.fail(why, "Allow the microphone, then hold again"));
            }
            Mic::Authorized | Mic::Unchecked => {}
        }
        let (home, s) = ((self.hooks.home)(), &self.settings);
        let mut needs = vec![("Recorder", s.recorder_path(&home), true), ("Engine", s.engine_bin(&home), true)];
        needs.extend(s.model_path(&home).map(|m| ("Model", m, false)));
        for (what, path, program) in needs {
            let ok = match (self.hooks.probe)(&path) {
                Probe::Program => true,
                Probe::File => !program,
                Probe::Missing => false,
            };
            if !ok {
                let why = format!("dictation: {} {} is missing (see flick dictation status)", what.to_lowercase(), path.display());
                return Err(self.fail(&why, &format!("{what} missing: see flick dictation status")));
            }
        }
        Ok(())
    }

    pub(super) fn stop(&mut self) -> Result<String, String> {
        if self.session.state() == State::Idle {
            // Key-up after an Esc cancel, or after a refused start.
            return Ok("dictation: not recording".into());
        }
        let now = (self.hooks.now)();
        let held_ms = self.live.started.map_or(0, |t| now.saturating_duration_since(t).as_millis() as u64);
        let effect = self.session.step(Input::Stop { held_ms })?;
        match (effect, self.live.recorder.take()) {
            (Effect::Transcribe, Some(recorder)) => {
                self.transcribe(recorder);
                Ok("dictation: transcribing".into())
            }
            (_, recorder) => {
                if let Some(recorder) = recorder {
                    recorder.discard();
                }
                (self.hooks.pill)(Pill::Hide);
                (self.hooks.log)(&format!("dictation: a {held_ms} ms hold, too short; discarded"));
                Ok("dictation: too short; nothing recorded".into())
            }
        }
    }

    fn transcribe(&mut self, recorder: Recorder) {
        let (s, epoch, home) = (&self.settings, self.session.epoch(), (self.hooks.home)());
        let wav = (self.hooks.cache)().join(clip::name(epoch));
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job {
            epoch,
            recorder,
            argv: engine::argv(s, &home, &wav),
            wav,
            silence_rms: s.silence_rms,
            trailing_space: s.trailing_space,
            budget: Duration::from_secs(u64::from(s.timeout_secs)),
            cancel: cancel.clone(),
        };
        worker::start(job, self.hooks.spawn, &self.inbox, self.hooks.notify);
        self.live.cancel = Some(cancel);
        self.live.target = (self.hooks.frontmost)();
        (self.hooks.pill)(Pill::Transcribing);
    }

    pub(super) fn cancel(&mut self) -> Result<String, String> {
        self.session.step(Input::Cancel)?;
        if let Some(recorder) = self.live.recorder.take() {
            recorder.discard();
        }
        if let Some(cancel) = self.live.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        (self.hooks.pill)(Pill::Hide);
        Ok("dictation: cancelled".into())
    }

    /// `ModuleChanged`: a queued Esc, a recorder that ended by itself, results, insertion.
    pub(super) fn wake(&mut self) {
        if (self.hooks.take_cancel)() {
            let _ = self.cancel();
        }
        if let Some(full) = self.live.recorder.as_ref().filter(|r| r.ended()).map(Recorder::full) {
            if full {
                let _ = self.stop();
            } else {
                let _ = self.cancel();
                self.fail("dictation: the recorder exited while recording", "The recorder stopped (see flick dictation status)");
            }
        }
        for done in worker::drain(&self.inbox) {
            self.finish(done);
        }
        if let Some(pending) = self.live.pending.take() {
            self.insert(pending);
        }
    }

    /// A worker result.
    fn finish(&mut self, done: Done) {
        let epoch = done.epoch;
        let input = match done.outcome {
            Outcome::Text(_) => Input::Transcribed { epoch },
            Outcome::Silent { .. } => Input::Silent { epoch },
            Outcome::Failed(_) | Outcome::Cancelled => Input::Failed { epoch },
        };
        let (audio, engine) = (done.audio_secs, done.engine_secs);
        match (self.session.step(input), done.outcome) {
            (Ok(Effect::Insert), Outcome::Text(text)) => {
                let chars = text.chars().count();
                (self.hooks.log)(&format!("dictation: {audio:.1} s of audio, engine {engine:.2} s, {chars} chars"));
                self.last = Some(text.trim_end().to_string());
                self.live.pending = Some((text, (self.hooks.now)() + RELEASE_CAP));
            }
            (Ok(Effect::Silent), Outcome::Silent { zeros: true }) => {
                self.fail("dictation: no audio from the microphone (denied or muted?)", "No audio: microphone denied or muted?");
            }
            (Ok(Effect::Silent), _) => {
                (self.hooks.log)(&format!("dictation: {audio:.1} s of audio, no speech"));
                (self.hooks.pill)(Pill::Result("No speech heard"));
            }
            (Ok(Effect::Failed), Outcome::Failed(e)) => {
                self.fail(&format!("dictation: {e}"), &e);
            }
            _ => {}
        }
    }

    /// Put `text` in, once the modifiers are up, unless the guards say no.
    fn insert(&mut self, (text, until): (String, Instant)) {
        if (self.hooks.flags)() & MODIFIERS != 0 && (self.hooks.now)() < until {
            self.live.pending = Some((text, until));
            (self.hooks.later)(RELEASE_POLL);
            return;
        }
        let refused = if (self.hooks.secure_input)() {
            Some("Secure input is on")
        } else if (self.hooks.frontmost)() != self.live.target {
            Some("Another app is in front")
        } else {
            None
        };
        if let Some(why) = refused {
            self.fail(&format!("dictation: not inserted: {why}"), &format!("{why}: kept, see flick dictation last"));
        } else {
            match self.settings.insert {
                Insert::Paste => (self.hooks.paste)(&text, self.settings.restore_ms),
                Insert::Type => (self.hooks.type_text)(&text),
            }
            (self.hooks.pill)(Pill::Result(text.trim_end()));
        }
        let _ = self.session.step(Input::Inserted);
    }

    /// The recorder ended by itself (tests wait for it).
    #[cfg(test)]
    pub(super) fn live_ended(&self) -> bool {
        self.live.recorder.as_ref().is_some_and(Recorder::ended)
    }

    /// `Started`: remove clips a crashed run left.
    pub(super) fn wipe(&self) {
        let n = clip::wipe(&(self.hooks.cache)());
        if n > 0 {
            (self.hooks.log)(&format!("dictation: removed {n} leftover clips"));
        }
    }

    /// Show `pill` as an error, log `why`, keep it for `status`; returns `why`.
    fn fail(&mut self, why: &str, pill: &str) -> String {
        (self.hooks.pill)(Pill::Error(pill));
        (self.hooks.log)(why);
        self.problem = Some(why.trim_start_matches("dictation: ").to_string());
        why.to_string()
    }
}
