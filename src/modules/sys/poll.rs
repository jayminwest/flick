//! The fleet's background work. The main thread only starts threads and reads `State`.
//!
//! - `sys-fleet-<i>`: one read of machine `i`, started by a round for each due machine
//!   that no read is running for (`Slot::busy`, per machine: a slow ssh machine holds only
//!   itself, flick-fd36). Each result lands as soon as it is in; clearing `busy` always
//!   wakes `Shared::wait`ers, even when the result was dropped.
//! - `sys-fleet-tick`: while the fleet view or window shows, at most one pending at a time;
//!   sleeps `VISIBLE_EVERY` s and posts, and the module's next poll starts the next one if it
//!   still shows. It, not a round, keeps a visible fleet polling, so a fleet of only
//!   `via = "local"` machines (no round at all) refreshes this Mac's cache too (flick-1e00).
//! - `sys-fleet-timer`: only with `refresh_secs > 0`; sleeps, posts, repeats until stopped.
//!
//! A round captures `Fleet::epoch`; a reload or sleep bumps it, so a read still in flight
//! (a wake from sleep, a changed machine list) drops its result (mx-82e6eb).

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::Value;

use super::fleet::Fetched;
use super::io::{self, Entry, Hooks, Shared};
use super::settings::{Machine, Via};
use super::{probe, report, ssh};
use crate::core::control::{Flags, Reply};

/// Budget of one ssh call (connect 5 s inside it).
pub const SSH_BUDGET: Duration = Duration::from_secs(10);

/// Read every remote machine whose last attempt ended `min_age` seconds ago or longer and
/// that is not being read already, each on its own thread. True when one started.
pub fn round(shared: &Arc<Shared>, min_age: u64, hooks: Hooks) -> bool {
    let now = (hooks.now)();
    // Held while the threads start, so none applies its result before it is marked busy.
    let mut st = shared.lock();
    let epoch = st.fleet.epoch;
    let mut started = false;
    for (i, machine) in st.fleet.due(now, min_age) {
        let sh = Arc::clone(shared);
        let spawned = thread::Builder::new().name(format!("sys-fleet-{i}")).spawn(move || {
            let got = fetch(&machine, hooks);
            let stored = sh.lock().fleet.apply(epoch, i, &machine, got, (hooks.now)());
            if stored {
                sh.changed(hooks);
            } else {
                sh.wake();
            }
        });
        st.fleet.slots[i].busy = spawned.is_ok();
        started |= spawned.is_ok();
    }
    started
}

/// Post once after `every`, unless a tick is pending already. The poll the post leads to
/// calls this again while the fleet still shows.
pub fn tick_after(shared: &Arc<Shared>, every: Duration, hooks: Hooks) {
    let mut st = shared.lock();
    if st.fleet.ticking {
        return;
    }
    let sh = Arc::clone(shared);
    let spawned = thread::Builder::new().name("sys-fleet-tick".into()).spawn(move || {
        thread::sleep(every);
        sh.lock().fleet.ticking = false;
        sh.changed(hooks);
    });
    st.fleet.ticking = spawned.is_ok();
}

/// Post every `every` until `stop_timer`.
pub fn start_timer(shared: &Arc<Shared>, every: Duration, hooks: Hooks) {
    let epoch = {
        let mut st = shared.lock();
        st.fleet.timer += 1;
        st.fleet.timer
    };
    let sh = Arc::clone(shared);
    let _ = thread::Builder::new().name("sys-fleet-timer".into()).spawn(move || {
        loop {
            thread::sleep(every);
            if sh.lock().fleet.timer != epoch {
                return;
            }
            (hooks.post)();
        }
    });
}

/// End the timer at its next wake.
pub fn stop_timer(shared: &Shared) {
    shared.lock().fleet.timer += 1;
}

/// Whether a peer's error means its Flick has no `sys snapshot` verb yet.
fn too_old(error: &str) -> bool {
    error.starts_with("unknown module \"sys\"") || error.starts_with("sys: unknown command")
}

/// One machine's snapshot: from its Flick, else (too old or unreachable) over ssh when it
/// has a target.
pub fn fetch(machine: &Machine, hooks: Hooks) -> Result<Fetched, String> {
    if machine.via == Via::Ssh {
        return over_ssh(machine, hooks);
    }
    let words = ["sys".to_string(), "snapshot".into()];
    let why = match (hooks.ask)(machine.flick_host(), &words, Flags { json: true, remote: true }) {
        Ok(Reply::Ok(snapshot @ Value::Object(_))) => {
            return Ok(Fetched { snapshot, source: Via::Flick, note: None });
        }
        Ok(Reply::Ok(other)) => return Err(format!("flick: unexpected reply {other}")),
        Ok(Reply::Error(e)) if too_old(&e) => "flick too old (no sys snapshot)".to_string(),
        Ok(Reply::Error(e)) => return Err(e),
        Err(e) => e,
    };
    if machine.ssh.is_none() {
        return Err(why);
    }
    match over_ssh(machine, hooks) {
        Ok(f) => Ok(Fetched { note: Some(why), ..f }),
        Err(e) => Err(format!("{why}; {e}")),
    }
}

/// The probe and the machine's checks over ssh; its http and tcp checks from this Mac.
fn over_ssh(machine: &Machine, hooks: Hooks) -> Result<Fetched, String> {
    let target = machine.ssh.as_deref().ok_or("no ssh target")?;
    let services = &machine.service;
    let exit = (hooks.run_input)(&ssh::argv(target), &ssh::script(services), SSH_BUDGET).map_err(|e| format!("ssh: {e}"))?;
    if let Some(why) = ssh::failure(&exit) {
        return Err(why);
    }
    let now = (hooks.now)();
    let snap = probe::parse(&exit.stdout, now);
    let remote = ssh::verdicts(&exit.stdout, services);
    let entries: Vec<Entry> = services
        .iter()
        .zip(remote)
        .map(|(service, verdict)| {
            let verdict = verdict.unwrap_or_else(|| io::check_one(service, hooks));
            Entry { service: service.clone(), verdict: Some(verdict), checked_at: Some((hooks.now)()) }
        })
        .collect();
    let snapshot = report::snapshot_json(&snap, None, &entries, (hooks.now)());
    Ok(Fetched { snapshot, source: Via::Ssh, note: None })
}

#[cfg(test)]
mod tests;
