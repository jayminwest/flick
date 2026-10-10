//! The real `Hooks`: file checks through `std::fs`, the home directory, and the microphone
//! check (none yet: flick-0654 adds `platform` authorization and wires it here).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::{Hooks, Mic, Probe};

pub static REAL: Hooks = Hooks { probe, mic, home };

/// What `path` is, following symlinks (Homebrew's binaries are links).
fn probe(path: &Path) -> Probe {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() && m.permissions().mode() & 0o111 != 0 => Probe::Program,
        Ok(m) if m.is_file() => Probe::File,
        _ => Probe::Missing,
    }
}

// TODO(flick-0654): return platform::mic's AVCaptureDevice audio authorization.
fn mic() -> Mic {
    Mic::Unchecked
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_programs_files_and_nothing() {
        assert_eq!(probe(Path::new("/bin/sh")), Probe::Program);
        assert_eq!(probe(Path::new("/etc/shells")), Probe::File);
        assert_eq!(probe(Path::new("/bin")), Probe::Missing);
        assert_eq!(probe(Path::new("/no/such/flick/path")), Probe::Missing);
        assert_eq!(mic(), Mic::Unchecked);
        assert!(home().is_absolute());
    }
}
