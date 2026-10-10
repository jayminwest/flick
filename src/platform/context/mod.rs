//! What the user is looking at, read when a window is summoned: the front app, its focused
//! window's title and its selection (`front`), and a screenshot of the display under the
//! pointer taken with the caller's own window out of the way (`shoot_display`).
//!
//! Generic: it knows no module or window. The caller decides which window to hide and what
//! to do with the PNG. Pure rules (selection filter, shot paths, errors) live in `rules`.

pub mod rules;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::capture::{self, Request, Shot, Target};
use rules::ShotError;

/// The app in front and what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Front {
    pub pid: i32,
    /// Display name ("Safari").
    pub app: String,
    /// "com.apple.Safari"; None for an app without one.
    pub bundle_id: Option<String>,
    /// The focused window's title; None without Accessibility, a focused window or a title.
    pub window: Option<String>,
    /// `ax::selected_text`, read only when asked for.
    pub selection: Option<String>,
}

/// Snapshot the front app: name, bundle id, focused window title and, with `selection`, the
/// selected text. None when no app is in front or Flick itself is. Call it before showing a
/// window of Flick's. AX reads are bounded by short messaging timeouts, so a hung app costs
/// well under a second, but this still blocks: summon time only, never a hot path.
#[cfg_attr(not(test), expect(dead_code, reason = "context attach in chat (flick-65bd) calls it"))]
pub fn front(selection: bool) -> Option<Front> {
    let pid = rules::other_app(super::workspace::frontmost_pid(), own_pid())?;
    let (bundle_id, app) = super::workspace::app_identity(pid)?;
    let window = super::ax::focused_window_title(pid);
    let selection = if selection { super::ax::selected_text(pid) } else { None };
    Some(Front { pid, app, bundle_id, window, selection })
}

fn own_pid() -> i32 {
    i32::try_from(std::process::id()).unwrap_or(-1)
}

/// Screenshot the display under the pointer to a fresh temp PNG, with the caller's window off
/// screen. Main thread only.
///
/// Order: check Screen Recording (without it the PNG would be black but for the wallpaper and
/// Flick's windows, so `work` gets `ShotError::NotPermitted` and nothing is hidden); resolve
/// the display under the pointer; call `hide` (e.g. `surface::hide`); then on a worker thread
/// wait `rules::SETTLE`, run screencapture silently without the cursor, queue `restore` on the
/// main thread (e.g. `surface::show`) and call `work` with the result on that worker, where
/// it may upload. `restore` runs only when `hide` ran, and before `work`, so an upload never
/// keeps the window hidden. `work` owns the file and must delete it; a failed shot leaves none.
#[expect(dead_code, reason = "context attach in chat (flick-65bd) calls it")]
pub fn shoot_display(
    hide: impl FnOnce(),
    restore: impl FnOnce() + Send + 'static,
    work: impl FnOnce(Result<Shot, ShotError>) + Send + 'static,
) {
    if !capture::permitted() {
        spawn(move || work(Err(ShotError::NotPermitted)));
        return;
    }
    let req = Request {
        target: Target::Display(capture::display_under_mouse()),
        path: next_path(&std::env::temp_dir()),
        cursor: false,
        shadow: false,
        sound: false,
    };
    hide();
    let restore = Restore(Some(restore));
    spawn(move || {
        std::thread::sleep(rules::SETTLE);
        let shot = capture::run(&req).map_err(ShotError::Capture);
        if shot.is_err() {
            // A partial or non-PNG file is not the caller's to clean up.
            let _ = std::fs::remove_file(&req.path);
        }
        drop(restore);
        work(shot);
    });
}

/// Queues the caller's `restore` on the main thread when dropped, so the hidden window comes
/// back even if the worker could not start or the capture panicked.
struct Restore<F: FnOnce() + Send + 'static>(Option<F>);

impl<F: FnOnce() + Send + 'static> Drop for Restore<F> {
    fn drop(&mut self) {
        if let Some(restore) = self.0.take() {
            super::events::on_main(restore);
        }
    }
}

/// A path for the next shot that no earlier shot of this or another Flick used.
fn next_path(dir: &Path) -> std::path::PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    rules::shot_path(dir, std::process::id(), stamp, N.fetch_add(1, Ordering::Relaxed))
}

fn spawn(job: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name("flick-context-shot".into()).spawn(job) {
        eprintln!("flick: screenshot thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_paths_are_fresh_temp_pngs() {
        let dir = std::env::temp_dir();
        let (a, b) = (next_path(&dir), next_path(&dir));
        assert_ne!(a, b);
        assert!(a.starts_with(&dir) && a.extension().is_some_and(|e| e == "png"), "{a:?}");
        assert!(!a.exists() && !b.exists());
    }

    /// Manual smoke: reads the real front app (name, window title, selection) and prints it.
    /// `cargo test -- --ignored front_reads_the_app_in_front --nocapture`
    #[test]
    #[ignore = "reads the real front app"]
    fn front_reads_the_app_in_front() {
        let front = front(true);
        println!("{front:?}");
        if let Some(f) = front {
            assert!(f.pid > 0 && !f.app.is_empty());
        }
    }
}
