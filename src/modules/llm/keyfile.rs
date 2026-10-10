//! A server's API key for curl, never in its argv (flick-d73a): a curl config file (`-K`)
//! that holds only `header = "Authorization: Bearer <key>"`. The file is mode 0600, created
//! new (never reused) in `flick-llm`, a 0700 directory under this user's temp dir
//! (`$TMPDIR`, itself private on macOS). `KeyFile` removes it when it drops: after curl is
//! reaped, when curl does not start, when the write fails, and on a panic. It never holds
//! prompt or reply text.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::transport::wipe_string;

/// Numbers each file, so two calls never share one.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Whether curl's config syntax keeps `key` inside its quoted header: printable ASCII with
/// no space, `"` or `\`.
pub fn good_key(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
}

/// A written key file; removed on drop.
#[derive(Debug)]
pub struct KeyFile {
    path: PathBuf,
}

impl KeyFile {
    /// Write `key`'s file under this user's temp dir.
    pub fn write(key: &str) -> Result<KeyFile, String> {
        Self::write_in(&std::env::temp_dir(), key)
    }

    fn write_in(base: &Path, key: &str) -> Result<KeyFile, String> {
        if !good_key(key) {
            return Err("llm: api_key must be printable ASCII without spaces, quotes or backslashes".into());
        }
        let dir = private_dir(base)?;
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("{}-{n}.curl", std::process::id()));
        let open = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path);
        let mut file = open.map_err(|e| format!("llm: cannot write the API key file: {e}"))?;
        // From here on the file is removed on every path.
        let written = KeyFile { path };
        let mut text = format!("header = \"Authorization: Bearer {key}\"\n");
        let wrote = file.write_all(text.as_bytes());
        wipe_string(&mut text);
        wrote.map_err(|e| format!("llm: cannot write the API key file: {e}"))?;
        Ok(written)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for KeyFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// `base/flick-llm`, made 0700 if missing. An existing one must be a real directory (not a
/// symlink) that only its owner can enter; one another user owns refuses the file anyway.
fn private_dir(base: &Path) -> Result<PathBuf, String> {
    let dir = base.join("flick-llm");
    match DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("llm: cannot make {}: {e}", dir.display())),
    }
    let meta = fs::symlink_metadata(&dir).map_err(|e| format!("llm: cannot read {}: {e}", dir.display()))?;
    if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
        return Err(format!("llm: {} is not a private directory", dir.display()));
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flick-keyfile-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_file_holds_only_the_header_is_private_and_goes_on_drop() {
        let b = base("write");
        let file = KeyFile::write_in(&b, "sk-abc.123").unwrap();
        let path = file.path().to_path_buf();
        assert_eq!(path.parent(), Some(b.join("flick-llm").as_path()));
        assert_eq!(fs::read_to_string(&path).unwrap(), "header = \"Authorization: Bearer sk-abc.123\"\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        let other = KeyFile::write_in(&b, "k").unwrap();
        assert_ne!(other.path(), path, "each call gets its own file");
        drop(file);
        assert!(!path.exists());
        drop(other);
        assert_eq!(fs::read_dir(b.join("flick-llm")).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&b);
    }

    #[test]
    fn keys_that_could_break_the_config_line_are_refused() {
        for key in ["", "a b", "a\"b", "a\\b", "a\nb", "ké"] {
            assert!(!good_key(key), "{key:?}");
            assert!(KeyFile::write(key).unwrap_err().contains("printable ASCII"));
        }
        assert!(good_key("sk-proj_AZaz09.~!#$%&'()*+,/:;<=>?@[]^`{|}"));
    }

    #[test]
    fn a_shared_or_symlinked_directory_is_refused() {
        let b = base("open");
        fs::create_dir(b.join("flick-llm")).unwrap();
        fs::set_permissions(b.join("flick-llm"), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(KeyFile::write_in(&b, "k").unwrap_err().ends_with("is not a private directory"));
        let l = base("link");
        std::os::unix::fs::symlink(&b, l.join("flick-llm")).unwrap();
        assert!(KeyFile::write_in(&l, "k").unwrap_err().ends_with("is not a private directory"));
        let missing = b.join("nope");
        assert!(KeyFile::write_in(&missing, "k").unwrap_err().starts_with("llm: cannot make"));
        let _ = fs::remove_dir_all(&b);
        let _ = fs::remove_dir_all(&l);
    }
}
