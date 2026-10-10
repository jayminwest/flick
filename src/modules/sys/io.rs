//! The module's background work. Every child process (the probe, curl, launchctl, pgrep,
//! configured commands) and every connect runs on a thread started here, with a time
//! budget; the main thread only reads `State` (a short `lock`) and, for the first answer
//! with an empty cache, waits on it for at most `FIRST_WAIT`.
//!
//! - `sys-probe`: one run of `probe::PROBE`, then the parsed snapshot into `State`.
//! - `sys-checks`: one round of every `[[sys.service]]`, one thread per service in
//!   parallel; each result lands as soon as it is in.
//! - the fleet's threads live in `poll.rs` and share `State`.
//!
//! Nothing here repeats on its own: a round starts only when a command or the fleet asks,
//! and at most once per `MIN_AGE` seconds, so an idle Flick runs no children and no timers. Each thread
//! posts `ModuleChanged` when it writes. A reload bumps `State::epoch`; a round from before
//! it drops its results.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::check::{self, Verdict};
use super::fleet::Fleet;
use super::jobs::Tail;
use super::probe::{self, Snapshot};
use super::run::Exit;
use super::settings::{Kind, Service};
use crate::core::control::{Flags, Reply};

/// Budget of the probe's child.
pub const PROBE_BUDGET: Duration = Duration::from_secs(2);
/// Budget of a curl, launchctl, pgrep or connect check (curl's own `-m 3`, plus spawn).
pub const CHECK_BUDGET: Duration = Duration::from_secs(4);
/// Budget of a configured command.
pub const COMMAND_BUDGET: Duration = Duration::from_secs(5);
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
    /// Connect to `host:port` with a timeout; how long it took (`run::connect`).
    pub connect: fn(&str, Duration) -> Result<Duration, String>,
    /// This user's uid, for launchd's `gui/<uid>` domain.
    pub uid: fn() -> Option<u32>,
    /// Run an argv with `input` on its stdin and a time budget (`run::run_input`; ssh).
    pub run_input: fn(&[String], &str, Duration) -> Result<Exit, String>,
    /// Ask a peer's Flick (`core::control::PeerHooks::ask`).
    pub ask: fn(&str, &[String], Flags) -> Result<Reply, String>,
    /// The launcher panel is on screen (main thread).
    pub visible: fn() -> bool,
    /// Open a URL in its app: `vnc://` in Screen Sharing, a dash in the browser (main
    /// thread).
    pub open: fn(&str),
}

/// One configured service and its last verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub service: Service,
    pub verdict: Option<Verdict>,
    /// When `verdict` was taken, unix seconds.
    pub checked_at: Option<u64>,
}

/// What the threads write and the main thread reads.
#[derive(Default)]
pub struct State {
    pub snapshot: Option<Snapshot>,
    /// Why the last probe failed; the previous snapshot stays.
    pub probe_error: Option<String>,
    pub probing: bool,
    pub services: Vec<Entry>,
    pub checking: bool,
    /// The `[[sys.machine]]` fleet (`poll.rs`).
    pub fleet: Fleet,
    /// The last tail asked for (`jobs.rs`).
    pub tail: Option<Tail>,
    /// The last restart's result and when it ended (`jobs.rs`).
    pub acted: Option<(String, u64)>,
    /// When the last services round started.
    checked_round: Option<u64>,
    epoch: u64,
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
    pub fn changed(&self, hooks: Hooks) {
        self.wake();
        (hooks.post)();
    }

    /// Wake waiters only.
    pub fn wake(&self) {
        self.changed.notify_all();
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

    /// Use `services`, in this order. A service that did not change keeps its verdict; a
    /// round in flight drops its results.
    pub fn set_services(&self, services: &[Service]) {
        let mut st = self.lock();
        let old = std::mem::take(&mut st.services);
        st.services = services
            .iter()
            .map(|s| {
                let kept = old.iter().find(|e| e.service == *s);
                kept.cloned().unwrap_or(Entry { service: s.clone(), verdict: None, checked_at: None })
            })
            .collect();
        st.epoch += 1;
        st.checking = false;
        st.checked_round = None;
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

/// Check every service on threads unless a round runs, there are none, or the last round
/// started less than `MIN_AGE` ago. True when a round started.
pub fn refresh_services(shared: &Arc<Shared>, hooks: Hooks) -> bool {
    let now = (hooks.now)();
    let (epoch, services) = {
        let mut st = shared.lock();
        let fresh = st.checked_round.is_some_and(|t| now.saturating_sub(t) < MIN_AGE);
        if st.services.is_empty() || st.checking || fresh {
            return false;
        }
        st.checking = true;
        st.checked_round = Some(now);
        let services: Vec<Service> = st.services.iter().map(|e| e.service.clone()).collect();
        (st.epoch, services)
    };
    let sh = Arc::clone(shared);
    let spawned = thread::Builder::new().name("sys-checks".into()).spawn(move || {
        let workers: Vec<_> = services
            .into_iter()
            .enumerate()
            .filter_map(|(i, service)| {
                let sh = Arc::clone(&sh);
                let name = format!("sys-check-{i}");
                thread::Builder::new()
                    .name(name)
                    .spawn(move || {
                        let verdict = check_one(&service, hooks);
                        let mut st = sh.lock();
                        // A reload changed the list meanwhile.
                        if st.epoch != epoch {
                            return;
                        }
                        if let Some(e) = st.services.get_mut(i).filter(|e| e.service == service) {
                            e.verdict = Some(verdict);
                            e.checked_at = Some((hooks.now)());
                        }
                        drop(st);
                        sh.changed(hooks);
                    })
                    .ok()
            })
            .collect();
        for worker in workers {
            let _ = worker.join();
        }
        let mut st = sh.lock();
        if st.epoch == epoch {
            st.checking = false;
        }
        drop(st);
        sh.changed(hooks);
    });
    if spawned.is_err() {
        shared.lock().checking = false;
        return false;
    }
    true
}

/// One service's check: the child or connect it needs, then its verdict.
pub fn check_one(service: &Service, hooks: Hooks) -> Verdict {
    let word = service.word().to_string();
    let argv = |words: &[&str]| words.iter().map(|w| (*w).to_string()).collect::<Vec<String>>();
    match service.kind {
        Kind::Http => {
            let curl = argv(&["/usr/bin/curl", "-sS", "-m", "3", "-o", "/dev/null", "-w", "%{http_code} %{time_total}", &word]);
            check::http((hooks.run)(&curl, CHECK_BUDGET), service.limits())
        }
        Kind::Tcp => check::tcp((hooks.connect)(&word, CHECK_BUDGET), service.limits()),
        Kind::Launchd => {
            let Some(uid) = (hooks.uid)() else { return Verdict::unknown("no uid for the launchd domain") };
            let domain = format!("gui/{uid}");
            let print = argv(&["/bin/launchctl", "print", &format!("{domain}/{word}")]);
            check::launchd((hooks.run)(&print, CHECK_BUDGET), &domain)
        }
        Kind::Process => check::process((hooks.run)(&argv(&["/usr/bin/pgrep", "-x", &word]), CHECK_BUDGET)),
        Kind::Command => check::command((hooks.run)(service.argv(), COMMAND_BUDGET), service.limits()),
    }
}

#[cfg(test)]
mod tests;
