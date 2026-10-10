//! The module's background work. Every child process runs on a thread started here, with
//! a time budget; the main thread only reads `State` (a short `lock`) and, for the first
//! answer with an empty cache, waits on it for at most `FIRST_WAIT`.
//!
//! - `sys-probe`: one run of `probe::PROBE`, then the parsed snapshot into `State`.
//!
//! Nothing repeats on its own: a run starts only when a command asks, and at most once
//! per `MIN_AGE` seconds, so an idle Flick runs no children and no timers. Each thread
//! posts `ModuleChanged` when it writes.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::probe::{self, Snapshot};
use super::run::Exit;

/// Budget of the probe's child.
pub const PROBE_BUDGET: Duration = Duration::from_secs(2);
/// A cached answer younger than this is not refreshed again.
pub const MIN_AGE: u64 = 5;

/// What the threads need from the outside world; tests swap in fakes (`testkit`).
#[derive(Clone, Copy)]
pub struct Hooks {
    /// Post `ModuleChanged` for this module (any thread).
    pub post: fn(),
    /// Unix seconds.
    pub now: fn() -> u64,
    /// Run an argv with a time budget (`run::run`).
    pub run: fn(&[String], Duration) -> Result<Exit, String>,
}

/// What the threads write and the main thread reads.
#[derive(Default)]
pub struct State {
    pub snapshot: Option<Snapshot>,
    /// Why the last probe failed; the previous snapshot stays.
    pub probe_error: Option<String>,
    pub probing: bool,
}

#[derive(Default)]
pub struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    /// A poisoned lock still holds usable data.
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wake waiters and post `ModuleChanged`.
    fn changed(&self, hooks: Hooks) {
        self.changed.notify_all();
        (hooks.post)();
    }

    /// Wait until `done` holds or `budget` runs out; whether it holds.
    pub fn wait(&self, budget: Duration, done: impl Fn(&State) -> bool) -> bool {
        let deadline = Instant::now() + budget;
        let mut state = self.lock();
        while !done(&state) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self.changed.wait_timeout(state, left).unwrap_or_else(PoisonError::into_inner).0;
        }
        true
    }

}

/// Run the probe on a thread unless one runs or the snapshot is younger than `MIN_AGE`.
/// True when one started.
pub fn refresh_probe(shared: &Arc<Shared>, hooks: Hooks) -> bool {
    let now = (hooks.now)();
    {
        let mut st = shared.lock();
        let fresh = st.snapshot.as_ref().is_some_and(|s| now.saturating_sub(s.at) < MIN_AGE);
        if st.probing || fresh {
            return false;
        }
        st.probing = true;
    }
    let sh = Arc::clone(shared);
    let spawned = thread::Builder::new().name("sys-probe".into()).spawn(move || {
        let argv = ["/bin/sh".to_string(), "-c".to_string(), probe::PROBE.to_string()];
        let out = (hooks.run)(&argv, PROBE_BUDGET);
        let at = (hooks.now)();
        let mut st = sh.lock();
        match out {
            Ok(exit) if !exit.stdout.trim().is_empty() => {
                st.snapshot = Some(probe::parse(&exit.stdout, at));
                st.probe_error = None;
            }
            Ok(_) => st.probe_error = Some("probe: no output".into()),
            Err(e) => st.probe_error = Some(format!("probe: {e}")),
        }
        st.probing = false;
        drop(st);
        sh.changed(hooks);
    });
    if spawned.is_err() {
        shared.lock().probing = false;
        return false;
    }
    true
}

#[cfg(test)]
mod tests;
