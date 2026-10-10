//! One dictation at a time, as a pure state machine: `Session::step` takes what happened
//! (a verb, a worker result) and returns what the module must do. It never touches the
//! recorder, the engine or the screen, so every transition is unit tested.
//!
//! ```text
//! Idle --Start--> Recording --Stop (held >= min)--> Transcribing --Transcribed--> Inserting --Inserted--> Idle
//!                  |  Stop (held < min), Cancel      |  Silent, Failed, Cancel
//!                  +--> Idle                         +--> Idle
//! ```
//!
//! Each `Start` opens a new epoch. Worker results carry the epoch they were started under;
//! one from an older epoch (the user cancelled, or started again) is `Stale` and dropped.

/// Where the current dictation is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Recording,
    Transcribing,
    Inserting,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Recording => "recording",
            State::Transcribing => "transcribing",
            State::Inserting => "inserting",
        }
    }
}

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// `dictation start` (chord down).
    Start,
    /// `dictation stop` (chord up, or the recorder hit `max_seconds`), after a hold of
    /// `held_ms` milliseconds.
    Stop { held_ms: u64 },
    /// `dictation cancel` (Esc on the pill).
    Cancel,
    /// The engine returned text that survived cleanup.
    Transcribed { epoch: u64 },
    /// The clip was silence, or cleanup left no text.
    Silent { epoch: u64 },
    /// The recorder or the engine failed, or ran out of time.
    Failed { epoch: u64 },
    /// The text went in (or was kept as `last` because focus moved).
    Inserted,
}

/// Why a recording ended without a transcription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discard {
    /// Held shorter than `min_hold_ms`: an accidental press.
    TooShort,
    Cancelled,
}

/// What the module must do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Start the recorder (under the epoch `Session::epoch` now returns).
    Record,
    /// Stop the recorder and throw the audio away.
    Discard(Discard),
    /// Stop the recorder and hand the audio to the engine.
    Transcribe,
    /// Kill the engine; its result, if any, will be `Stale`.
    Abort,
    /// Put the transcript into the focused field.
    Insert,
    /// Done: nothing to insert (silence) or an error to show.
    Silent,
    Failed,
    /// Back to idle after an insertion.
    Done,
    /// A result from an earlier epoch: drop it, change nothing.
    Stale,
}

#[derive(Debug, Default)]
pub struct Session {
    state: State,
    epoch: u64,
    min_hold_ms: u64,
}

impl Session {
    pub fn new(min_hold_ms: u32) -> Self {
        Session { min_hold_ms: u64::from(min_hold_ms), ..Session::default() }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// The current dictation's epoch: worker results must carry it.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// A new `min_hold_ms` (config reload); applies from the next `Stop`.
    pub fn set_min_hold(&mut self, min_hold_ms: u32) {
        self.min_hold_ms = u64::from(min_hold_ms);
    }

    /// Apply `input`. `Err` says why it does not apply now (the state is unchanged); verbs
    /// answer with it, chord edges may ignore it.
    pub fn step(&mut self, input: Input) -> Result<Effect, String> {
        use State::{Idle, Inserting, Recording, Transcribing};
        let (next, effect) = match (self.state, input) {
            (Idle, Input::Start) => {
                self.epoch += 1;
                (Recording, Effect::Record)
            }
            (Recording, Input::Stop { held_ms }) if held_ms < self.min_hold_ms => {
                (Idle, Effect::Discard(Discard::TooShort))
            }
            (Recording, Input::Stop { .. }) => (Transcribing, Effect::Transcribe),
            (Recording, Input::Cancel) => (Idle, Effect::Discard(Discard::Cancelled)),
            (Transcribing, Input::Cancel) => {
                // Results already under way are now stale.
                self.epoch += 1;
                (Idle, Effect::Abort)
            }
            (_, Input::Transcribed { epoch } | Input::Silent { epoch } | Input::Failed { epoch })
                if epoch != self.epoch || self.state != Transcribing =>
            {
                return Ok(Effect::Stale);
            }
            (Transcribing, Input::Transcribed { .. }) => (Inserting, Effect::Insert),
            (Transcribing, Input::Silent { .. }) => (Idle, Effect::Silent),
            (Transcribing, Input::Failed { .. }) => (Idle, Effect::Failed),
            (Inserting, Input::Inserted) => (Idle, Effect::Done),
            (state, input) => return Err(refusal(state, input)),
        };
        self.state = next;
        Ok(effect)
    }
}

/// Why `input` does not apply in `state`.
fn refusal(state: State, input: Input) -> String {
    match input {
        Input::Start => format!("dictation: already {}", state.name()),
        Input::Stop { .. } => "dictation: not recording".into(),
        Input::Cancel if state == State::Idle => "dictation: nothing to cancel".into(),
        Input::Cancel => format!("dictation: too late to cancel ({})", state.name()),
        _ => format!("dictation: {input:?} while {}", state.name()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `inputs` from a fresh session, returning each result and the end state.
    fn run(inputs: &[Input]) -> (Vec<Result<Effect, String>>, State) {
        let mut s = Session::new(300);
        let out = inputs.iter().map(|i| s.step(*i)).collect();
        (out, s.state())
    }

    const STOP: Input = Input::Stop { held_ms: 2000 };

    #[test]
    fn a_dictation_runs_start_to_insert() {
        let mut s = Session::new(300);
        assert_eq!((s.state(), s.epoch()), (State::Idle, 0));
        assert_eq!(s.step(Input::Start), Ok(Effect::Record));
        assert_eq!((s.state(), s.epoch()), (State::Recording, 1));
        assert_eq!(s.step(STOP), Ok(Effect::Transcribe));
        assert_eq!(s.state(), State::Transcribing);
        assert_eq!(s.step(Input::Transcribed { epoch: 1 }), Ok(Effect::Insert));
        assert_eq!(s.state(), State::Inserting);
        assert_eq!(s.step(Input::Inserted), Ok(Effect::Done));
        assert_eq!(s.state(), State::Idle);
        // The next one opens a new epoch.
        assert_eq!(s.step(Input::Start), Ok(Effect::Record));
        assert_eq!(s.epoch(), 2);
    }

    #[test]
    fn short_holds_and_cancels_record_nothing() {
        let (out, end) = run(&[Input::Start, Input::Stop { held_ms: 299 }]);
        assert_eq!(out[1], Ok(Effect::Discard(Discard::TooShort)));
        assert_eq!(end, State::Idle);
        let (out, end) = run(&[Input::Start, Input::Stop { held_ms: 300 }]);
        assert_eq!((out[1].clone(), end), (Ok(Effect::Transcribe), State::Transcribing));
        let (out, end) = run(&[Input::Start, Input::Cancel]);
        assert_eq!((out[1].clone(), end), (Ok(Effect::Discard(Discard::Cancelled)), State::Idle));
        let mut s = Session::new(300);
        s.set_min_hold(0);
        s.step(Input::Start).unwrap();
        assert_eq!(s.step(Input::Stop { held_ms: 0 }), Ok(Effect::Transcribe));
    }

    #[test]
    fn cancel_while_transcribing_makes_the_result_stale() {
        let mut s = Session::new(300);
        s.step(Input::Start).unwrap();
        s.step(STOP).unwrap();
        assert_eq!(s.step(Input::Cancel), Ok(Effect::Abort));
        assert_eq!((s.state(), s.epoch()), (State::Idle, 2));
        assert_eq!(s.step(Input::Transcribed { epoch: 1 }), Ok(Effect::Stale));
        assert_eq!(s.state(), State::Idle);
        // A new dictation ignores the old one's late result too.
        s.step(Input::Start).unwrap();
        s.step(STOP).unwrap();
        assert_eq!(s.step(Input::Failed { epoch: 1 }), Ok(Effect::Stale));
        assert_eq!(s.step(Input::Silent { epoch: 3 }), Ok(Effect::Silent));
        assert_eq!(s.state(), State::Idle);
    }

    #[test]
    fn failures_and_silence_end_the_dictation() {
        let (out, end) = run(&[Input::Start, STOP, Input::Failed { epoch: 1 }]);
        assert_eq!((out[2].clone(), end), (Ok(Effect::Failed), State::Idle));
        let (out, end) = run(&[Input::Start, STOP, Input::Silent { epoch: 1 }]);
        assert_eq!((out[2].clone(), end), (Ok(Effect::Silent), State::Idle));
        // A result while recording (none can be under way) is stale, not a transition.
        let (out, end) = run(&[Input::Start, Input::Transcribed { epoch: 1 }]);
        assert_eq!((out[1].clone(), end), (Ok(Effect::Stale), State::Recording));
    }

    #[test]
    fn inputs_out_of_turn_are_refused_and_change_nothing() {
        let err = |inputs: &[Input]| {
            let (out, end) = run(inputs);
            (out.last().unwrap().clone().unwrap_err(), end)
        };
        assert_eq!(err(&[STOP]), ("dictation: not recording".into(), State::Idle));
        assert_eq!(err(&[Input::Cancel]), ("dictation: nothing to cancel".into(), State::Idle));
        assert_eq!(err(&[Input::Inserted]), ("dictation: Inserted while idle".into(), State::Idle));
        let rec = err(&[Input::Start, Input::Start]);
        assert_eq!(rec, ("dictation: already recording".into(), State::Recording));
        let busy = err(&[Input::Start, STOP, Input::Start]);
        assert_eq!(busy, ("dictation: already transcribing".into(), State::Transcribing));
        let late = err(&[Input::Start, STOP, Input::Transcribed { epoch: 1 }, Input::Cancel]);
        assert_eq!(late, ("dictation: too late to cancel (inserting)".into(), State::Inserting));
        let stop = err(&[Input::Start, STOP, STOP]);
        assert_eq!(stop, ("dictation: not recording".into(), State::Transcribing));
    }

    #[test]
    fn states_have_names() {
        let names = [State::Idle, State::Recording, State::Transcribing, State::Inserting].map(State::name);
        assert_eq!(names, ["idle", "recording", "transcribing", "inserting"]);
    }
}
