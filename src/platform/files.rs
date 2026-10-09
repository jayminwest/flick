//! Moving files to the Trash (`NSFileManager`). This is Flick's only way to remove a file:
//! nothing calls `remove_file` or `remove_dir_all`, so every removal can be put back.

use std::path::{Path, PathBuf};

use objc2_foundation::{NSFileManager, NSString, NSURL};

/// Move the file, folder or bundle at `path` to the Trash, like Finder's Move to Trash.
/// `Ok` holds its new location in the Trash; `Err` holds the system's description of why it
/// could not move (missing, no permission, protected container).
pub fn trash(path: &Path) -> Result<PathBuf, String> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.display().to_string()));
    let mut moved = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut moved))
        .map_err(|e| e.localizedDescription().to_string())?;
    moved.and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string())).ok_or_else(|| {
        format!("{} moved to the Trash, but its new location is unknown", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flk-{}-trash-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn moves_a_file_to_the_trash_and_reports_where() {
        let dir = scratch("file");
        let file = dir.join("flick-trash-test.txt");
        std::fs::write(&file, b"flick").unwrap();

        let moved = trash(&file).unwrap();
        assert!(!file.exists());
        assert_eq!(std::fs::read(&moved).unwrap(), b"flick");
        assert!(moved.components().any(|c| c.as_os_str() == ".Trash"), "{}", moved.display());

        // Take it back out so the test leaves nothing in the user's Trash.
        std::fs::rename(&moved, &file).unwrap();
        assert!(file.exists());
    }

    #[test]
    fn a_missing_path_fails_with_the_system_text() {
        let missing = scratch("missing").join("does-not-exist");
        let err = trash(&missing).unwrap_err();
        assert!(!err.is_empty());
    }
}
