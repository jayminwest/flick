//! The module's background threads. They own every herdr call; the main thread only reads
//! `Shared` (a `lock`, never a join) and starts threads.
//!
//! - `herdr-local`: ping, `sync` (list + subscription), then one blocking `next` per event
//!   until the server goes away. A pane the fleet does not know means `sync` again; the
//!   new subscription starts before the old one drops. On EOF the machine is marked not
//!   live and the thread ends; the module starts it again on the next launcher open, view
//!   open or wake. Never a retry loop.
//! - `herdr-remote`: one round lists every due remote machine in parallel (one thread per
//!   machine, `herdr --machine`), then optionally sleeps and posts once more, so a visible
//!   launcher keeps polling and a hidden one stops.
//! - `herdr-timer`: only with `remote_refresh_secs > 0`; sleeps that long, posts, repeats.
//! - one-shot threads: machine discovery, an agent's output, a jump.
//!
//! Every thread reports through `Hooks::post` (`Event::ModuleChanged { module: "herdr" }`).
//! Agent output stays in memory (`Shared::preview`); nothing here writes a file or logs it.

use super::local::{Closer, Local, Pong, Subscription};
use super::model::{self, Fleet};
use super::remote::Remote;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

/// The fleet's name for the herdr server on this Mac.
pub const LOCAL: &str = "local";

/// Lines asked of `agent.read`; the preview keeps the last `preview_lines` non-empty ones.
pub const READ_LINES: u32 = 40;

/// What threads need from the outside world; tests swap in no-ops.
#[derive(Clone, Copy)]
pub struct Hooks {
    /// Post `ModuleChanged` for this module (any thread).
    pub post: fn(),
    /// Unix seconds.
    pub now: fn() -> u64,
    /// The launcher panel is on screen (main thread).
    pub visible: fn() -> bool,
    /// Bring terminal app `name` to the front (any thread; it hops to the main thread).
    pub front: fn(String),
    /// App `name` is frontmost (main thread).
    pub is_front: fn(&str) -> bool,
    /// Post a notification `(id, title, body)` (main thread).
    pub notify: fn(&str, &str, &str),
    /// Route notification clicks to `clicks` and ask for permission when `ask` (main
    /// thread, `Started` and reloads).
    pub listen: fn(ask: bool),
    /// Ids of the notifications clicked since the last call (main thread).
    pub clicks: fn() -> Vec<String>,
    /// Whether notifications can show: `on`, `not permitted`, ... (main thread).
    pub notifications: fn() -> String,
}

/// One agent's output, for the detail view. In memory only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub machine: String,
    pub pane_id: String,
    /// `None` while it loads.
    pub lines: Option<Result<Vec<String>, String>>,
}

/// Facts for `flick herdr status`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// The local server's last `ping`.
    pub pong: Option<Pong>,
    /// Why `herdr machine list` failed, when it did.
    pub discovery: Option<String>,
    /// The last jump's error; cleared by a good jump.
    pub jump: Option<String>,
}

#[derive(Default)]
struct LocalCtl {
    /// Bumped by `stop_local`; a thread whose generation is old stops writing and exits.
    epoch: u64,
    /// The generation of the running thread.
    running: Option<u64>,
    closer: Option<Closer>,
}

/// State the threads write and the main thread reads.
#[derive(Default)]
pub struct Shared {
    pub fleet: Mutex<Fleet>,
    pub info: Mutex<Info>,
    pub preview: Mutex<Option<Preview>>,
    local: Mutex<LocalCtl>,
    remote_busy: AtomicBool,
    timer_gen: AtomicU64,
    /// Bumped when the machine list changes, so a late discovery does not overwrite it.
    machines_gen: AtomicU64,
}

/// A poisoned lock still holds usable data: a panicking thread never leaves the fleet
/// half-written in a way that matters more than losing it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Use `machines` as the fleet, in this order; a discovery in flight is dropped.
    pub fn set_machines(&self, machines: &[String]) {
        self.machines_gen.fetch_add(1, Ordering::Relaxed);
        lock(&self.fleet).set_machines(machines.iter().cloned());
    }

    fn current(&self, epoch: u64) -> bool {
        lock(&self.local).epoch == epoch
    }
}

/// Start the local thread unless one is running.
pub fn start_local(shared: &Arc<Shared>, local: Local, hooks: Hooks) {
    let epoch = {
        let mut c = lock(&shared.local);
        if c.running == Some(c.epoch) {
            return;
        }
        c.running = Some(c.epoch);
        c.epoch
    };
    let sh = Arc::clone(shared);
    let spawned = thread::Builder::new().name("herdr-local".into()).spawn(move || {
        run_local(&sh, &local, epoch, hooks);
        let mut c = lock(&sh.local);
        if c.running == Some(epoch) {
            c.running = None;
        }
    });
    if spawned.is_err() {
        lock(&shared.local).running = None;
    }
}

/// End the local thread: its subscription closes and it exits without marking an error.
pub fn stop_local(shared: &Shared) {
    let mut c = lock(&shared.local);
    c.epoch += 1;
    c.running = None;
    if let Some(closer) = c.closer.take() {
        closer.close();
    }
}

fn run_local(shared: &Shared, local: &Local, epoch: u64, hooks: Hooks) {
    let fail = |e: String| {
        if shared.current(epoch) {
            let mut fleet = lock(&shared.fleet);
            fleet.set_live(LOCAL, false);
            fleet.apply_error(LOCAL, e, (hooks.now)());
            drop(fleet);
            (hooks.post)();
        }
    };
    match local.ping() {
        Ok(pong) => {
            if let Some(warning) = pong.mismatch() {
                eprintln!("flick: herdr: {warning}");
            }
            lock(&shared.info).pong = Some(pong);
        }
        Err(e) => return fail(e),
    }
    let mut old: Option<Subscription> = None;
    loop {
        let (agents, mut sub) = match local.sync(LOCAL) {
            Ok(synced) => synced,
            Err(e) => return fail(e),
        };
        drop(old.take());
        {
            let mut c = lock(&shared.local);
            if c.epoch != epoch {
                return;
            }
            match sub.closer() {
                Ok(closer) => c.closer = Some(closer),
                Err(e) => {
                    drop(c);
                    return fail(e);
                }
            }
        }
        {
            let mut fleet = lock(&shared.fleet);
            fleet.set_live(LOCAL, true);
            fleet.apply_list(LOCAL, agents, (hooks.now)());
        }
        (hooks.post)();
        let relist = follow(shared, &mut sub, epoch, hooks);
        if !shared.current(epoch) {
            return;
        }
        if !relist {
            return fail("herdr disconnected".into());
        }
        old = Some(sub);
    }
}

/// Apply `sub`'s events until it ends (false) or a pane appears that needs a new list
/// (true).
fn follow(shared: &Shared, sub: &mut Subscription, epoch: u64, hooks: Hooks) -> bool {
    while let Some(event) = sub.next() {
        let Some(change) = model::parse_event(LOCAL, &event) else { continue };
        let applied = {
            let mut fleet = lock(&shared.fleet);
            if !shared.current(epoch) {
                return true;
            }
            fleet.apply_event(LOCAL, change, (hooks.now)())
        };
        if applied.changed {
            (hooks.post)();
        }
        if applied.relist {
            return true;
        }
    }
    false
}

/// List every remote machine checked `min_age` seconds ago or longer, in parallel, unless
/// a round runs. `again`: after the round, sleep that long and post once, so the module
/// can decide on another round. True when a round started.
pub fn poll_remote(
    shared: &Arc<Shared>,
    remote: &Remote,
    min_age: u64,
    again: Option<Duration>,
    hooks: Hooks,
) -> bool {
    let now = (hooks.now)();
    let due: Vec<String> = {
        let fleet = lock(&shared.fleet);
        let names = fleet.machine_names();
        names
            .into_iter()
            .filter(|m| *m != LOCAL && fleet.needs_refresh(m, now, min_age))
            .map(str::to_string)
            .collect()
    };
    if due.is_empty() || shared.remote_busy.swap(true, Ordering::AcqRel) {
        return false;
    }
    let (sh, remote) = (Arc::clone(shared), remote.clone());
    let spawned = thread::Builder::new().name("herdr-remote".into()).spawn(move || {
        let workers: Vec<_> = due
            .into_iter()
            .filter_map(|machine| {
                let (sh, remote) = (Arc::clone(&sh), remote.clone());
                let name = format!("herdr-{machine}");
                thread::Builder::new()
                    .name(name)
                    .spawn(move || {
                        let listed = remote.list(&machine);
                        let mut fleet = lock(&sh.fleet);
                        // A reload dropped the machine meanwhile.
                        if fleet.machine(&machine).is_none() {
                            return;
                        }
                        match listed {
                            Ok(agents) => fleet.apply_list(&machine, agents, (hooks.now)()),
                            Err(e) => fleet.apply_error(&machine, e, (hooks.now)()),
                        };
                        drop(fleet);
                        (hooks.post)();
                    })
                    .ok()
            })
            .collect();
        for worker in workers {
            let _ = worker.join();
        }
        sh.remote_busy.store(false, Ordering::Release);
        if let Some(wait) = again {
            thread::sleep(wait);
            (hooks.post)();
        }
    });
    if spawned.is_err() {
        shared.remote_busy.store(false, Ordering::Release);
        return false;
    }
    true
}

/// Post every `every` until `stop_timer` (opt-in background refresh of remote machines).
pub fn start_timer(shared: &Arc<Shared>, every: Duration, hooks: Hooks) {
    let epoch = shared.timer_gen.fetch_add(1, Ordering::AcqRel) + 1;
    let sh = Arc::clone(shared);
    let _ = thread::Builder::new().name("herdr-timer".into()).spawn(move || {
        loop {
            thread::sleep(every);
            if sh.timer_gen.load(Ordering::Acquire) != epoch {
                return;
            }
            (hooks.post)();
        }
    });
}

/// End the timer at its next wake.
pub fn stop_timer(shared: &Shared) {
    shared.timer_gen.fetch_add(1, Ordering::AcqRel);
}

/// The fleet becomes `local` plus herdr's enabled saved machines. On failure the fleet
/// stays as it is and `Info::discovery` says why.
pub fn discover(shared: &Arc<Shared>, remote: &Remote, hooks: Hooks) {
    let epoch = shared.machines_gen.load(Ordering::Acquire);
    let (sh, remote) = (Arc::clone(shared), remote.clone());
    let _ = thread::Builder::new().name("herdr-machines".into()).spawn(move || {
        let found = remote.machines();
        if sh.machines_gen.load(Ordering::Acquire) != epoch {
            return;
        }
        match found {
            Ok(labels) => {
                let mut names = vec![LOCAL.to_string()];
                names.extend(labels.into_iter().filter(|l| l != LOCAL));
                lock(&sh.fleet).set_machines(names);
                lock(&sh.info).discovery = None;
            }
            Err(e) => lock(&sh.info).discovery = Some(e),
        }
        (hooks.post)();
    });
}

/// Where an agent's calls go.
#[derive(Clone)]
pub struct Transport {
    pub local: Local,
    pub remote: Remote,
}

/// Load `pane_id`'s output on `machine` into `Shared::preview`, keeping `lines` lines.
pub fn fetch_preview(
    shared: &Arc<Shared>,
    t: &Transport,
    machine: &str,
    pane_id: &str,
    lines: usize,
    hooks: Hooks,
) {
    let want = Preview { machine: machine.into(), pane_id: pane_id.into(), lines: None };
    *lock(&shared.preview) = Some(want.clone());
    let (sh, t) = (Arc::clone(shared), t.clone());
    let _ = thread::Builder::new().name("herdr-read".into()).spawn(move || {
        let text = if want.machine == LOCAL {
            t.local.read(&want.pane_id, READ_LINES)
        } else {
            t.remote.read(&want.machine, &want.pane_id, READ_LINES)
        };
        let mut preview = lock(&sh.preview);
        // Another agent's view opened meanwhile: drop this one.
        if preview.as_ref().is_some_and(|p| p.machine == want.machine && p.pane_id == want.pane_id)
        {
            *preview = Some(Preview { lines: Some(text.map(|t| model::preview(&t, lines))), ..want });
        }
        drop(preview);
        (hooks.post)();
    });
}

/// Focus `pane_id` in herdr on `machine` and bring `terminal` to the front: after the
/// focus for the local server (milliseconds), before it for a remote one (an SSH round
/// trip), so the terminal never waits on SSH.
pub fn jump(
    shared: &Arc<Shared>,
    t: &Transport,
    machine: &str,
    pane_id: &str,
    terminal: &str,
    hooks: Hooks,
) {
    let (sh, t) = (Arc::clone(shared), t.clone());
    let (machine, pane_id, terminal) = (machine.to_string(), pane_id.to_string(), terminal.to_string());
    let _ = thread::Builder::new().name("herdr-jump".into()).spawn(move || {
        let focused = if machine == LOCAL {
            let focused = t.local.focus(&pane_id);
            (hooks.front)(terminal);
            focused
        } else {
            (hooks.front)(terminal);
            t.remote.focus(&machine, &pane_id)
        };
        let error = focused.err().map(|e| format!("jump to {machine}/{pane_id}: {e}"));
        if let Some(e) = &error {
            eprintln!("flick: herdr: {e}");
        }
        lock(&sh.info).jump = error;
    });
}

#[cfg(test)]
mod tests;
