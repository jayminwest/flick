//! Hiding windows without minimizing them, for `tile`'s workspaces: `hide` parks each window at
//! a bottom corner of its screen (`corner`), `place` puts windows at given frames (back where
//! they were). Each returns a `Batch` that tells when every app thread it went to has applied
//! it. `exists` and `stray` serve the restore after a crash.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::corner::{self, Area};
use super::{WinId, ids, with_session};
use crate::platform::{Rect, screens};

/// Frame sets sent to app threads: done once each of those threads has run them, or ended
/// (its app quit) without.
pub struct Batch(Arc<Acks>);

struct Acks {
    sent: Instant,
    state: Mutex<State>,
    done: Condvar,
    notify: fn(),
}

struct State {
    /// App threads that have not answered, plus one while the batch is still being sent.
    left: usize,
    finished: Option<Instant>,
}

impl Batch {
    fn new(notify: fn()) -> Batch {
        let state = Mutex::new(State { left: 0, finished: None });
        Batch(Arc::new(Acks { sent: Instant::now(), state, done: Condvar::new(), notify }))
    }

    /// A batch with nothing to wait for: done at once, for tests.
    #[cfg(test)]
    pub fn done() -> Batch {
        let batch = Batch::new(|| {});
        drop(Ack::new(&batch.0));
        batch
    }

    /// A batch whose app never answers, for tests of callers that wait.
    #[cfg(test)]
    pub fn pending() -> Batch {
        let batch = Batch::new(|| {});
        std::mem::forget(Ack::new(&batch.0));
        batch
    }

    /// When the last app thread answered; None while one has not.
    pub fn finished(&self) -> Option<Instant> {
        self.0.lock().finished
    }

    /// Wait at most `max` for every app thread; how long the batch took, None on a timeout.
    pub fn wait(&self, max: Duration) -> Option<Duration> {
        let state = self.0.lock();
        let (state, _) = self
            .0
            .done
            .wait_timeout_while(state, max, |s| s.finished.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        state.finished.map(|at| at - self.0.sent)
    }
}

impl Acks {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One answer a batch waits for, given when dropped: after its job ran, or with the job when
/// the app thread had already ended.
struct Ack(Arc<Acks>);

impl Ack {
    fn new(acks: &Arc<Acks>) -> Ack {
        acks.lock().left += 1;
        Ack(Arc::clone(acks))
    }
}

impl Drop for Ack {
    fn drop(&mut self) {
        let last = {
            let mut state = self.0.lock();
            state.left -= 1;
            let last = state.left == 0;
            if last {
                state.finished = Some(Instant::now());
            }
            last
        };
        if last {
            self.0.done.notify_all();
            (self.0.notify)();
        }
    }
}

/// Move windows to `frames` (`set_frames`); `notify` (any thread) hears when every app
/// thread has applied them. Unknown ids are skipped.
pub fn place(frames: &[(WinId, Rect)], notify: fn()) -> Batch {
    super::set_frames(frames);
    let batch = Batch::new(notify);
    let sending = Ack::new(&batch.0);
    with_session(|s| {
        let mut pids: Vec<i32> = frames.iter().filter_map(|&(id, _)| s.shared.pid_of(id)).collect();
        pids.sort_unstable();
        pids.dedup();
        for app in pids.iter().filter_map(|pid| s.apps.get(pid)) {
            // Runs after the frame-set job `set_frames` sent: jobs run in order.
            let ack = Ack::new(&batch.0);
            app.send(move |_| drop(ack));
        }
    });
    drop(sending);
    batch
}

/// Park each window `(id, its frame now)` at the hide corner of its screen, keeping its size.
pub fn hide(windows: &[(WinId, Rect)], notify: fn()) -> Batch {
    place(&corner::park(windows, &areas()), notify)
}

/// Where to put a window at `frame` that sits parked at a screen corner: centred on that
/// screen. None for any other window.
pub fn stray(frame: Rect) -> Option<Rect> {
    corner::rescue(frame, &areas())
}

/// The window server still has window `id`. Needs no Accessibility permission.
pub fn exists(id: WinId) -> bool {
    ids::level(id).is_some()
}

fn areas() -> Vec<Area> {
    screens::all().into_iter().map(|s| Area { frame: s.frame, visible: s.visible }).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NOTIFIED: AtomicUsize = AtomicUsize::new(0);

    fn notify() {
        NOTIFIED.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn a_batch_is_done_when_its_last_ack_drops() {
        let batch = Batch::new(notify);
        let (a, b) = (Ack::new(&batch.0), Ack::new(&batch.0));
        let before = NOTIFIED.load(Ordering::SeqCst);
        drop(a);
        assert_eq!(batch.finished(), None);
        assert_eq!(batch.wait(Duration::from_millis(1)), None);
        let waiter = std::thread::spawn(move || drop(b));
        assert!(batch.wait(Duration::from_secs(15)).is_some());
        waiter.join().unwrap();
        assert!(batch.finished().is_some());
        assert_eq!(NOTIFIED.load(Ordering::SeqCst), before + 1);
        assert!(Batch::done().wait(Duration::ZERO).is_some());
    }

    #[test]
    fn placing_without_a_session_is_done_at_once() {
        let rect = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
        assert!(place(&[(1, rect)], || {}).finished().is_some());
        assert!(!exists(0));
    }
}
