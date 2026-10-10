//! The real `Hooks`: file checks through `std::fs`, the home directory, and the microphone
//! authorization through `platform::mic`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::{Hooks, Mic, Probe};
use crate::platform::mic::{self, Access};

pub static REAL: Hooks = Hooks { probe, mic, home };

/// What `path` is, following symlinks (Homebrew's binaries are links).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_programs_files_and_nothing() {
        assert_eq!(probe(Path::new("/bin/sh")), Probe::Program);
        assert_eq!(probe(Path::new("/etc/shells")), Probe::File);
        assert_eq!(probe(Path::new("/bin")), Probe::Missing);
        assert_eq!(probe(Path::new("/no/such/flick/path")), Probe::Missing);
        assert!(home().is_absolute());
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
}
