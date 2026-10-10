//! Test-only temp directories that remove themselves. Each `Scratch` is a fresh, empty
//! `flk-<pid>-<name>-<n>` directory under the system temp dir; `n` keeps tests that run in
//! parallel with the same name apart. Dropping it deletes the directory and everything in it,
//! so test runs leave nothing in `$TMPDIR`.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("flk-{}-{name}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        Scratch(dir)
    }
}

impl Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        self
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_fresh_unique_and_gone_on_drop() {
        let (a, b) = (Scratch::new("scratch"), Scratch::new("scratch"));
        assert_ne!(*a, *b);
        std::fs::write(a.join("f"), "x").unwrap();
        assert!(a.is_dir() && std::fs::read_dir(&b).unwrap().next().is_none());
        let path = a.to_path_buf();
        drop(a);
        assert!(!path.exists());
    }
}
