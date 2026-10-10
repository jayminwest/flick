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
//! This step holds the settings, the session state machine (`session`), the audio and
//! transcript helpers (`audio`, `transcript`) and `dictation status`. The recorder and
//! engine workers (flick-8b7c) and the start/stop/cancel/last verbs with insertion
//! (flick-a085) build on them.

mod audio;
mod session;
mod settings;
mod transcript;
mod wire;

use std::path::{Path, PathBuf};

use session::Session;
use settings::{Engine, Insert, Settings};

use crate::config::Section;
use crate::core::{Cx, Module, unknown_verb};

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
#[cfg_attr(not(test), expect(dead_code, reason = "the platform check (flick-0654) answers the others"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mic {
    Authorized,
    Denied,
    Restricted,
    NotDetermined,
    /// Not checked: this build has no platform check yet (flick-0654).
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

/// What the module asks of the system: `wire::REAL` in the app, fakes in tests.
pub struct Hooks {
    pub probe: fn(&Path) -> Probe,
    /// The microphone authorization. The hook point for flick-0654's
    /// `platform` check; `wire::REAL` answers `Mic::Unchecked` until then.
    pub mic: fn() -> Mic,
    /// The home directory, for `~/` in paths.
    pub home: fn() -> PathBuf,
}

pub struct Dictation {
    /// `[dictation]` set at least one key.
    on: bool,
    settings: Settings,
    session: Session,
    hooks: &'static Hooks,
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
        Dictation { on: false, settings, session, hooks }
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
        "dictation status"
    }

    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        match args {
            [verb] if verb == "status" => Ok(self.status()),
            _ => Err(unknown_verb("dictation", args)),
        }
    }
}

#[cfg(test)]
mod tests;
