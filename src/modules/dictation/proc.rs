//! The two programs dictation runs (the recorder and the engine), seen through a small
//! interface: `Hooks::spawn` starts one with its stdout piped, and `Child` stops and reaps
//! it. `wire.rs` backs it with `std::process`; tests back it with scripted fakes.

use std::io::Read;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// A running program.
pub trait Child: Send {
    /// Ask it to stop (SIGTERM): `rec` flushes its buffer and closes stdout.
    fn terminate(&mut self);
    /// Stop it now (SIGKILL) and reap it.
    fn kill(&mut self);
    /// `Some(success)` once it has exited (and is reaped), `None` while it runs.
    fn try_wait(&mut self) -> Result<Option<bool>, String>;
}

/// A started program: its stdout and its handle.
pub struct Spawned {
    pub stdout: Box<dyn Read + Send>,
    pub child: Box<dyn Child>,
}

/// Start `argv` (program, then arguments) with stdin and stderr closed and stdout piped.
pub type Spawn = fn(&[String]) -> Result<Spawned, String>;

/// The child, shared between the thread that reads it and the one that stops it.
pub type Shared = Mutex<Box<dyn Child>>;

pub fn lock(child: &Shared) -> MutexGuard<'_, Box<dyn Child>> {
    child.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The last path component of `argv[0]`, for messages.
pub fn name(argv: &[String]) -> &str {
    let program = argv.first().map_or("", String::as_str);
    program.rsplit('/').next().unwrap_or(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_program() {
        let argv = |s: &[&str]| s.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        assert_eq!(name(&argv(&["/opt/homebrew/bin/whisper-cli", "-m"])), "whisper-cli");
        assert_eq!(name(&argv(&["stt"])), "stt");
        assert_eq!(name(&[]), "");
    }
}
