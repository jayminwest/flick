//! The real `Hooks`: file checks through `std::fs`, programs through `std::process`, and
//! the microphone, pill, pasteboard, key events, timers and frontmost app through
//! `platform`.

use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Instant;

use super::proc::{Child, Spawned};
use super::{Hooks, Mic, Pill, Probe};
use crate::core::Event;
use crate::platform::mic::{self, Access};
use crate::platform::pill::{self, Phase};
use crate::platform::{events, keytap, pasteboard, timer, workspace};

pub static REAL: Hooks = Hooks {
    probe,
    mic,
    ask_mic: || mic::request(|_granted| {}),
    home,
    cache: || home().join("Library/Caches/Flick/dictation"),
    spawn,
    notify,
    later: |secs| timer::after(secs, notify),
    arm,
    take_cancel: || CANCEL.swap(false, Ordering::AcqRel),
    now: Instant::now,
    pill: show,
    frontmost: workspace::frontmost_pid,
    flags: keytap::current_flags,
    secure_input: keytap::secure_input,
    paste,
    type_text: keytap::type_text,
    log: |line| eprintln!("flick: {line}"),
};

/// What a path is, following symlinks (Homebrew's binaries are links).
fn probe(path: &Path) -> Probe {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() && m.permissions().mode() & 0o111 != 0 => Probe::Program,
        Ok(m) if m.is_file() => Probe::File,
        _ => Probe::Missing,
    }
}

fn mic() -> Mic {
    from_access(mic::status())
}

fn from_access(access: Option<Access>) -> Mic {
    match access {
        Some(Access::Authorized) => Mic::Authorized,
        Some(Access::Denied) => Mic::Denied,
        Some(Access::Restricted) => Mic::Restricted,
        Some(Access::NotDetermined) => Mic::NotDetermined,
        None => Mic::Unchecked,
    }
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn notify() {
    events::post(Event::ModuleChanged { module: "dictation" });
}

/// Esc on the pill while recording or transcribing (`pill::on_cancel`).
static CANCEL: AtomicBool = AtomicBool::new(false);

/// Route the pill's Esc to the module: set a flag and wake it.
fn arm() {
    pill::on_cancel(|| {
        CANCEL.store(true, Ordering::Release);
        notify();
    });
}

/// A program started by `spawn`.
struct Proc(std::process::Child);

impl Child for Proc {
    fn terminate(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            // std has no kill(2) with a signal; /bin/kill sends SIGTERM.
            let pid = self.0.id().to_string();
            let _ = Command::new("/bin/kill").args(["-TERM", &pid]).stderr(Stdio::null()).status();
        }
    }

    fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }

    fn try_wait(&mut self) -> Result<Option<bool>, String> {
        self.0.try_wait().map(|s| s.map(|s| s.success())).map_err(|e| e.to_string())
    }
}

fn spawn(argv: &[String]) -> Result<Spawned, String> {
    let (program, rest) = argv.split_first().ok_or("dictation: no program")?;
    let mut child = Command::new(program)
        .args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err(format!("{program}: no stdout"));
    };
    Ok(Spawned { stdout: Box::new(stdout), child: Box::new(Proc(child)) })
}

thread_local! {
    /// The recorder's level while the pill shows `Recording` (main thread).
    static LEVEL: RefCell<Option<Arc<AtomicU32>>> = const { RefCell::new(None) };
}

fn level() -> f32 {
    LEVEL.with_borrow(|l| l.as_ref().map_or(0.0, |a| f32::from_bits(a.load(Ordering::Relaxed))))
}

fn show(p: Pill<'_>) {
    if !matches!(p, Pill::Recording(_)) {
        LEVEL.set(None);
    }
    match p {
        Pill::Recording(source) => {
            LEVEL.set(Some(source));
            pill::show(Phase::Recording(level));
        }
        Pill::Transcribing => pill::show(Phase::Transcribing),
        Pill::Result(text) => pill::show(Phase::Result(text)),
        Pill::Error(text) => pill::show(Phase::Error(text)),
        Pill::Hide => pill::hide(),
    }
}

/// Paste `text` as a transient, concealed pasteboard item with cmd+V, and put the old
/// contents back after `restore_ms`, unless something else wrote the pasteboard meanwhile.
fn paste(text: &str, restore_ms: u32) {
    let saved = pasteboard::snapshot();
    let ours = pasteboard::set_transient_text(text);
    keytap::paste();
    timer::after(f64::from(restore_ms) / 1000.0, move || {
        if pasteboard::change_count() == ours {
            pasteboard::restore(&saved);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::time::Duration;

    use super::*;

    #[test]
    fn probes_programs_files_and_nothing() {
        assert_eq!(probe(Path::new("/bin/sh")), Probe::Program);
        assert_eq!(probe(Path::new("/etc/shells")), Probe::File);
        assert_eq!(probe(Path::new("/bin")), Probe::Missing);
        assert_eq!(probe(Path::new("/no/such/flick/path")), Probe::Missing);
        assert!(home().is_absolute());
        assert!((REAL.cache)().ends_with("Library/Caches/Flick/dictation"));
    }

    #[test]
    fn microphone_access_maps_to_status() {
        let pairs = [
            (Some(Access::Authorized), Mic::Authorized),
            (Some(Access::Denied), Mic::Denied),
            (Some(Access::Restricted), Mic::Restricted),
            (Some(Access::NotDetermined), Mic::NotDetermined),
            (None, Mic::Unchecked),
        ];
        for (access, want) in pairs {
            assert_eq!(from_access(access), want);
        }
        // The real check answers (it never prompts).
        assert_ne!(mic(), Mic::Unchecked);
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn spawns_reads_and_stops_programs() {
        let Spawned { mut stdout, mut child } = spawn(&argv(&["/bin/echo", "hi"])).unwrap();
        let mut out = String::new();
        stdout.read_to_string(&mut out).unwrap();
        assert_eq!(out, "hi\n");
        let since = Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(since.elapsed() < Duration::from_secs(5));
        }
        assert_eq!(child.try_wait(), Ok(Some(true)));
        child.terminate();

        let Spawned { mut child, .. } = spawn(&argv(&["/bin/sleep", "30"])).unwrap();
        assert_eq!(child.try_wait(), Ok(None));
        child.terminate();
        let since = Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(since.elapsed() < Duration::from_secs(5), "SIGTERM stops it");
        }
        assert_eq!(child.try_wait(), Ok(Some(false)));

        let Spawned { mut child, .. } = spawn(&argv(&["/bin/sleep", "30"])).unwrap();
        child.kill();
        assert_eq!(child.try_wait(), Ok(Some(false)));

        assert_eq!(spawn(&[]).err().unwrap(), "dictation: no program");
        assert!(spawn(&argv(&["/no/such/flick/rec"])).err().unwrap().starts_with("/no/such/flick/rec: "));
    }

    #[test]
    fn the_level_follows_the_recording() {
        assert!(level().abs() < f32::EPSILON);
        LEVEL.set(Some(Arc::new(AtomicU32::new(0.5f32.to_bits()))));
        assert!((level() - 0.5).abs() < f32::EPSILON);
        LEVEL.set(None);
        assert!(!(REAL.take_cancel)());
    }
}
