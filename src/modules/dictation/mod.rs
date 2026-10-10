//! Module `dictation`: hold a chord, speak, release, and the transcript goes into the
//! focused field. Everything stays on this Mac: the recorder (`rec`) and the engine
//! (whisper.cpp's `whisper-cli` or `parakeet-cli`, or a `command`) are local programs, and
//! the module has no network code. The whole module is in `NET_DENIED`, so a peer can never
//! start the microphone.
//!
//! Off until `[dictation]` sets at least one key; a config without the table runs nothing.
//! The trigger is a `[[keys.chord]]` with `{ flick = "dictation start" }` / `"dictation
//! stop"` actions: modules never import each other, so the keys module sends verbs here.
//!
//! - `session`: the pure state machine; `flow`: the verbs and the main-thread steps
//!   (pill, insertion and its guards).
//! - `recorder`: `rec` and its reader thread (audio in memory, level, `max_seconds`).
//! - `worker`: one transcription on a thread: silence gate, the clip (`clip`), the engine
//!   (`engine`) within `timeout_secs`, cleanup (`transcript`); results come back through an
//!   inbox and `ModuleChanged { module: "dictation" }`.
//! - Every program and macOS call goes through `Hooks` (`wire::REAL`; fakes in tests).
//!
//! Transcripts and audio are never logged or stored: log lines carry durations and character
//! counts, `last` lives in memory only, and the clip is deleted after every engine run.

mod audio;
mod clip;
mod engine;
#[cfg(test)]
mod fake;
mod flow;
mod proc;
mod recorder;
mod session;
mod settings;
mod transcript;
mod wire;
mod worker;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Instant;

use flow::Live;
use session::Session;
use settings::{Engine, Insert, Settings};

use crate::config::Section;
use crate::core::{Cx, Event, Module, unknown_verb};

/// What a path is, for `status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Missing,
    /// A file without an execute bit.
    File,
    /// An executable file.
    Program,
}

/// Microphone authorization (`AVCaptureDevice` for audio), for `status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mic {
    Authorized,
    Denied,
    Restricted,
    NotDetermined,
    /// Not checked: `AVCaptureDevice` is missing or gave an unknown answer.
    Unchecked,
}

impl Mic {
    fn text(self) -> &'static str {
        match self {
            Mic::Authorized => "authorized",
            Mic::Denied => "denied (System Settings > Privacy & Security > Microphone)",
            Mic::Restricted => "restricted by a profile",
            Mic::NotDetermined => "not determined (macOS asks at the first dictation)",
            Mic::Unchecked => "not checked",
        }
    }
}

/// What the pill shows (`platform::pill`).
pub enum Pill<'a> {
    /// Recording, with the level the recorder publishes (`f32` bits, 0..=1).
    Recording(Arc<AtomicU32>),
    Transcribing,
    Result(&'a str),
    Error(&'a str),
    Hide,
}

/// What the module asks of the system: `wire::REAL` in the app, fakes in tests.
pub struct Hooks {
    pub probe: fn(&Path) -> Probe,
    /// The microphone authorization (`platform::mic::status` in `wire::REAL`).
    pub mic: fn() -> Mic,
    /// Ask for microphone access (the system prompt shows while not determined).
    pub ask_mic: fn(),
    /// The home directory, for `~/` in paths.
    pub home: fn() -> PathBuf,
    /// Where the clip goes while the engine runs.
    pub cache: fn() -> PathBuf,
    /// Start a program (recorder or engine); called on worker threads too.
    pub spawn: proc::Spawn,
    /// Post `ModuleChanged { module: "dictation" }`, from any thread.
    pub notify: fn(),
    /// Post it after this many seconds (main thread).
    pub later: fn(f64),
    /// Once, before the first recording: Esc on the pill queues a cancel.
    pub arm: fn(),
    /// Esc was pressed on the pill since the last call.
    pub take_cancel: fn() -> bool,
    pub now: fn() -> Instant,
    pub pill: fn(Pill<'_>),
    pub frontmost: fn() -> Option<i32>,
    /// The physical modifier flags (`CGEventFlags`).
    pub flags: fn() -> u64,
    pub secure_input: fn() -> bool,
    /// Paste the text (transient pasteboard item, cmd+V), restoring the pasteboard after
    /// this many milliseconds.
    pub paste: fn(&str, u32),
    pub type_text: fn(&str),
    /// One log line (never a transcript).
    pub log: fn(&str),
}

pub struct Dictation {
    /// `[dictation]` set at least one key.
    on: bool,
    settings: Settings,
    session: Session,
    hooks: &'static Hooks,
    /// The running dictation's recorder, engine and pending text.
    live: Live,
    inbox: worker::Inbox,
    /// The last transcript (memory only), for `dictation last`.
    last: Option<String>,
    /// Why the last dictation went wrong, for `status`.
    problem: Option<String>,
    /// `Hooks::arm` ran.
    armed: bool,
}

impl Default for Dictation {
    fn default() -> Self {
        Dictation::with(&wire::REAL)
    }
}

impl Dictation {
    fn with(hooks: &'static Hooks) -> Self {
        let settings = Settings::default();
        let session = Session::new(settings.min_hold_ms);
        let (live, inbox) = (Live::default(), worker::Inbox::default());
        Dictation { on: false, settings, session, hooks, live, inbox, last: None, problem: None, armed: false }
    }

    /// `flick dictation status`: on or off, the state, and whether each program and file
    /// the engine needs is there.
    fn status(&self) -> String {
        let s = &self.settings;
        let home = (self.hooks.home)();
        let probe = |path: &Path| match (self.hooks.probe)(path) {
            Probe::Program => "found",
            Probe::File => "not executable",
            Probe::Missing => "missing",
        };
        let program = |path: PathBuf| format!("{} ({})", path.display(), probe(&path));
        let on = if self.on { "on" } else { "off: set a key in [dictation] to turn it on" };
        let mut lines = vec![
            format!("dictation: {on}"),
            format!("state: {}", self.session.state().name()),
            format!("engine: {} {}", s.engine.name(), program(s.engine_bin(&home))),
        ];
        if let Some(model) = s.model_path(&home) {
            let found = match (self.hooks.probe)(&model) {
                Probe::Missing => "missing; Flick never downloads models, see docs/dictation.md",
                Probe::File | Probe::Program => "found",
            };
            lines.push(format!("model: {} ({found})", model.display()));
        }
        lines.push(format!("recorder: {}", program(s.recorder_path(&home))));
        lines.push(format!("microphone: {}", (self.hooks.mic)().text()));
        let restore = match s.insert {
            Insert::Paste => format!(" (clipboard restored after {} ms)", s.restore_ms),
            Insert::Type => String::new(),
        };
        lines.push(format!("insert: {}{restore}", s.insert.name()));
        if s.engine == Engine::Command {
            lines.push(format!("command: {}", s.command.join(" ")));
        }
        if let Some(problem) = &self.problem {
            lines.push(format!("last error: {problem}"));
        }
        lines.join("\n")
    }
}

impl Module for Dictation {
    fn id(&self) -> &'static str {
        "dictation"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let on = !table.get::<toml::Table>()?.is_empty();
        let settings = table.get::<Settings>()?;
        settings.check()?;
        self.session.set_min_hold(settings.min_hold_ms);
        (self.on, self.settings) = (on, settings);
        Ok(())
    }

    fn verbs(&self) -> &'static str {
        "dictation start | dictation stop | dictation cancel | dictation last | dictation status"
    }

    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        match args {
            [verb] if verb == "status" => Ok(self.status()),
            [verb] if verb == "start" => self.start(),
            [verb] if verb == "stop" => self.stop(),
            [verb] if verb == "cancel" => self.cancel(),
            [verb] if verb == "last" => self.last.clone().ok_or_else(|| "dictation: nothing dictated yet".into()),
            _ => Err(unknown_verb("dictation", args)),
        }
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        match event {
            Event::Started if self.on => self.wipe(),
            Event::ModuleChanged { module: "dictation" } => self.wake(),
            _ => {}
        }
        false
    }
}

#[cfg(test)]
mod tests;
