//! The one file dictation writes: the clip as a WAV for the engine, in a private directory
//! (`~/Library/Caches/Flick/dictation/`, 0700; the file 0600). It exists only while the
//! engine runs: `Clip` deletes it when dropped, whatever the outcome (text, error, time-out,
//! cancel, panic), and `wipe` removes leftovers of a crashed run at `Started`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The clip file; deleted on drop.
#[derive(Debug)]
pub struct Clip(PathBuf);

impl Clip {
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Clip {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// The clip's name for dictation `epoch` of this process.
pub fn name(epoch: u64) -> String {
    format!("{}-{epoch}.wav", std::process::id())
}

/// Write `bytes` to `path` (0600), creating its directory (0700) if needed. A file already
/// there (a leftover with the same name) is replaced.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<Clip> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    let _ = fs::remove_file(path);
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    let clip = Clip(path.to_path_buf());
    file.write_all(bytes)?;
    Ok(clip)
}

/// Remove every `.wav` in `dir`; how many went. A missing directory is fine.
pub fn wipe(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav") && fs::remove_file(p).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flick-dictation-clip-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_clip_is_private_and_gone_once_dropped() {
        let dir = dir("write");
        let path = dir.join(name(7));
        assert!(path.ends_with(format!("{}-7.wav", std::process::id())));
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, b"old").unwrap();
        let clip = write(&path, b"RIFF").unwrap();
        assert_eq!(clip.path(), path);
        assert_eq!(fs::read(&path).unwrap(), b"RIFF");
        assert_eq!((mode(&dir), mode(&path)), (0o700, 0o600));
        drop(clip);
        assert!(!path.exists());
        fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn wipe_removes_only_clips() {
        let dir = dir("wipe");
        assert_eq!(wipe(&dir), 0);
        fs::create_dir_all(&dir).unwrap();
        for f in ["1-1.wav", "2-9.wav", "notes.txt"] {
            fs::write(dir.join(f), b"x").unwrap();
        }
        assert_eq!(wipe(&dir), 2);
        assert!(dir.join("notes.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
        // A parent that is a file cannot hold the directory.
        assert!(write(Path::new("/dev/null/x.wav"), b"").is_err());
    }
}
