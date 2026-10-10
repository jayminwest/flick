//! The pure rules behind `context`: which selections count, where a display shot goes, and
//! why a shot failed. No `AppKit`; `ax` and `context` apply them.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::platform::capture;

/// How long the shutter waits after the caller's `hide`, so the window server has removed
/// the hidden window from the screen before screencapture reads it.
pub const SETTLE: Duration = Duration::from_millis(150);

/// A focused element with this `AXSubrole` is a password field: never read from it.
pub fn is_secure(subrole: Option<&str>) -> bool {
    subrole == Some("AXSecureTextField")
}

/// The selection worth attaching: `text` as the app gave it, or None when it is missing,
/// empty or only whitespace.
pub fn selection(text: Option<String>) -> Option<String> {
    text.filter(|t| !t.trim().is_empty())
}

/// The front app's pid unless it is `own` (Flick itself has nothing to attach).
pub fn other_app(front: Option<i32>, own: i32) -> Option<i32> {
    front.filter(|&pid| pid != own)
}

/// A fresh PNG path in `dir` for a display shot: unique per process (`pid`), launch time
/// (`stamp`, e.g. milliseconds since the epoch) and shot number `n`.
pub fn shot_path(dir: &Path, pid: u32, stamp: u128, n: u64) -> PathBuf {
    dir.join(format!("flick-shot-{pid}-{stamp}-{n}.png"))
}

/// Why a display shot gave no PNG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShotError {
    /// Screen Recording is not granted: screencapture would hand back only the wallpaper and
    /// Flick's own windows, so no shot is taken.
    NotPermitted,
    /// screencapture failed.
    Capture(capture::Error),
}

impl fmt::Display for ShotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPermitted => f.write_str(
                "Screen Recording is off for Flick: allow it in System Settings > Privacy & \
                 Security > Screen Recording, then restart Flick",
            ),
            Self::Capture(e) => write!(f, "screenshot failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_secure_text_fields_are_secure() {
        assert!(is_secure(Some("AXSecureTextField")));
        for other in [None, Some(""), Some("AXTextField"), Some("AXSearchField")] {
            assert!(!is_secure(other), "{other:?}");
        }
    }

    #[test]
    fn a_selection_needs_more_than_whitespace() {
        assert_eq!(selection(None), None);
        assert_eq!(selection(Some(String::new())), None);
        assert_eq!(selection(Some(" \n\t ".into())), None);
        // Kept exactly as selected, surrounding whitespace included.
        assert_eq!(selection(Some("  a b\n".into())).as_deref(), Some("  a b\n"));
    }

    #[test]
    fn flick_in_front_is_no_app() {
        assert_eq!(other_app(Some(42), 7), Some(42));
        assert_eq!(other_app(Some(7), 7), None);
        assert_eq!(other_app(None, 7), None);
    }

    #[test]
    fn shot_paths_are_distinct_pngs_in_the_dir() {
        let dir = Path::new("/tmp/x");
        let a = shot_path(dir, 12, 1_700_000_000_000, 0);
        assert_eq!(a, Path::new("/tmp/x/flick-shot-12-1700000000000-0.png"));
        assert_ne!(a, shot_path(dir, 12, 1_700_000_000_000, 1));
        assert_ne!(a, shot_path(dir, 13, 1_700_000_000_000, 0));
        assert_ne!(a, shot_path(dir, 12, 1_700_000_000_001, 0));
    }

    #[test]
    fn shot_errors_say_what_to_do() {
        let denied = ShotError::NotPermitted.to_string();
        assert!(denied.contains("Screen Recording") && denied.contains("restart"), "{denied}");
        let failed = ShotError::Capture(capture::Error::Failed("boom".into())).to_string();
        assert_eq!(failed, "screenshot failed: boom");
    }
}
