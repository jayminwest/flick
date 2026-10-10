//! The module's background work. Every child (herdr, curl) runs on a thread started here,
//! with a time budget; the main thread only reads `Poll` (a short `lock`) and starts
//! threads.
//!
//! - `kota-poll`: one round. herdr's `agent list` and curl's `/ok` run in parallel (curl
//!   on `kota-dash`); the round's `Seen` goes through the debounce (`Presence::apply`)
//!   under the lock, then the thread posts `ModuleChanged`.
//! - `kota-timer`: sleeps until the next round is due, then posts `ModuleChanged`; the
//!   main thread (`tick`) starts the round and arms the next timer. One timer at a time:
//!   arming bumps `Poll::timer`, and an older timer that wakes sees it and exits.
//! - `kota-clock`: while the menu bar item shows, posts `ModuleChanged` every
//!   `CLOCK_SECS` (not while asleep), so the item's age text (`KOTA: idle · 4m`) moves
//!   between rounds (flick-c3eb). `Poll::clock` names the live one; another value retires it.
//!
//! Sleep and lock (`suspend`) bump `Poll::epoch`: a round in flight drops its result, the
//! timer retires, the presence is marked stale. Wake and unlock (`resume`) wait
//! `WAKE_DELAY`, then run a round; failures in the grace period after keep the last state.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use super::ask::Ask;
use super::presence::{self, Presence, Round, Transition};
use super::run::{self, Exit};
use super::settings::Settings;

/// Budget of the herdr child (one SSH round trip, usually 2-3 s).
pub const HERDR_BUDGET: Duration = Duration::from_secs(10);
/// curl's own `--max-time`.
pub const CURL_MAX_TIME: u64 = 5;
/// Budget of the curl child: its `--max-time` plus spawn.
pub const CURL_BUDGET: Duration = Duration::from_secs(CURL_MAX_TIME + 2);
/// After a wake or unlock, the network gets this long before the first round.
pub const WAKE_DELAY: u64 = 5;
/// `kota refresh` (and menu opens) start at most one round per this many seconds.
pub const REFRESH_EVERY: u64 = 10;
/// The menu bar item's age text changes at most once a minute.
pub const CLOCK_SECS: u64 = 60;

/// What the threads need from the outside world; tests swap in fakes (`testkit`).
#[derive(Clone, Copy)]
pub struct Hooks {
    /// Post `ModuleChanged` for this module (any thread).
    pub post: fn(),
    /// Unix seconds.
    pub now: fn() -> u64,
    /// Run an argv with a time budget (`run::run`).
    pub run: fn(&[String], Duration) -> Result<Exit, String>,
    /// Run an argv with a text on its stdin and a time budget (`run::feed`).
    pub feed: fn(&[String], &str, Duration) -> Result<Exit, String>,
    /// This binary's path (`run::exe`).
    pub exe: fn() -> Result<String, String>,
    /// This Mac's short host name (`run::host`).
    pub host: fn() -> Option<String>,
    /// Sleep on a timer thread.
    pub sleep: fn(Duration),
    /// The local UTC offset at a unix time, for `Checked 12:03`.
    pub utc_offset: fn(i64) -> i32,
}

/// What the threads write and the main thread reads.
#[derive(Debug, Default)]
pub struct Poll {
    pub presence: Presence,
    /// A round runs.
    pub running: bool,
    /// When the last round started and ended, unix seconds (any epoch).
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    /// Bumped by sleep, lock and settings changes: rounds from before drop their results.
    pub epoch: u64,
    /// Bumped each time a timer is armed or retired.
    pub timer: u64,
    /// When the armed timer fires, unix seconds.
    pub timer_due: Option<u64>,
    /// Asleep or locked: no timed rounds.
    pub asleep: bool,
    /// The last wake or unlock.
    pub woke_at: Option<u64>,
    /// No timed round before this (the wake delay).
    pub not_before: Option<u64>,
    /// Poll fast until this (after an ask, flick-039d).
    pub fast_until: Option<u64>,
    /// State changes not yet seen by the main thread (`item.rs` notifies on down).
    pub transitions: Vec<Transition>,
    /// The last asks, newest first (`ask.rs`).
    pub asks: Vec<Ask>,
    /// The live `kota-clock`, by number; `None`: none runs.
    pub clock: Option<u64>,
    /// Clocks started so far: numbers each one.
    pub clocks: u64,
}

#[derive(Default)]
pub struct Shared {
    poll: Mutex<Poll>,
}

impl Shared {
    /// A poisoned lock still holds usable data.
    pub fn lock(&self) -> MutexGuard<'_, Poll> {
        self.poll.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Settings changed: drop a round in flight, retire the timer, forget the presence
    /// (it was about another pane or server).
    pub fn reset(&self) {
        let mut p = self.lock();
        let (epoch, timer, asks) = (p.epoch + 1, p.timer + 1, std::mem::take(&mut p.asks));
        // The clock is the item's, not the target's: it keeps running.
        let (clock, clocks) = (p.clock, p.clocks);
        *p = Poll { epoch, timer, asks, clock, clocks, ..Poll::default() };
    }

    /// Retire the timer and every thread at its next check (module dropped or disabled).
    pub fn stop(&self) {
        let mut p = self.lock();
        p.epoch += 1;
        p.timer += 1;
        p.running = false;
        p.timer_due = None;
        p.clock = None;
    }
}

/// The argvs of one round: herdr's `agent list` and curl's `/ok`.
pub fn argvs(s: &Settings, herdr: &str, host: Option<&str>) -> (Vec<String>, Vec<String>) {
    let mut list = vec![herdr.to_string()];
    if !s.is_local(host) {
        list.extend(["--machine".to_string(), s.machine.clone()]);
    }
    list.extend(["agent".to_string(), "list".to_string()]);
    let max = CURL_MAX_TIME.to_string();
    let curl = ["/usr/bin/curl", "-sS", "--fail", "--max-time", &max, &s.ok_url()];
    (list, curl.map(str::to_string).to_vec())
}

/// Start a round unless one runs. True when one started.
pub fn round(shared: &Arc<Shared>, settings: &Settings, hooks: Hooks) -> bool {
    let epoch = {
        let mut p = shared.lock();
        if p.running {
            return false;
        }
        p.running = true;
        p.started_at = Some((hooks.now)());
        p.epoch
    };
    let (sh, s) = (Arc::clone(shared), settings.clone());
    let spawned = thread::Builder::new().name("kota-poll".into()).spawn(move || {
        let (list, curl) = argvs(&s, &run::resolve(&s.herdr), (hooks.host)().as_deref());
        let dash = thread::Builder::new().name("kota-dash".into()).spawn(move || (hooks.run)(&curl, CURL_BUDGET));
        let herdr = presence::herdr_result((hooks.run)(&list, HERDR_BUDGET));
        let dash = match dash {
            Ok(worker) => worker.join().unwrap_or_else(|_| Err("curl thread panicked".into())),
            Err(e) => Err(format!("no thread for curl: {e}")),
        };
        let seen = presence::observe(&Round { herdr, dash: presence::dash_result(dash) }, &s.cwd, &s.pane);
        let now = (hooks.now)();
        let mut p = sh.lock();
        if p.epoch != epoch {
            return;
        }
        let grace = presence::in_grace(now, p.woke_at);
        if let Some(t) = p.presence.apply(seen, now, grace) {
            p.transitions.push(t);
        }
        p.running = false;
        p.ended_at = Some(now);
        drop(p);
        (hooks.post)();
    });
    if spawned.is_err() {
        shared.lock().running = false;
        return false;
    }
    true
}

/// Arm the timer: post `ModuleChanged` in `secs`. Replaces an armed timer.
pub fn arm(shared: &Arc<Shared>, secs: u64, hooks: Hooks) {
    let armed = {
        let mut p = shared.lock();
        p.timer += 1;
        p.timer_due = Some((hooks.now)().saturating_add(secs));
        p.timer
    };
    let sh = Arc::clone(shared);
    let _ = thread::Builder::new().name("kota-timer".into()).spawn(move || {
        (hooks.sleep)(Duration::from_secs(secs));
        let mut p = sh.lock();
        if p.timer != armed {
            return;
        }
        p.timer_due = None;
        drop(p);
        (hooks.post)();
    });
}

/// Start the minute clock unless one runs (the menu bar item shows).
pub fn clock(shared: &Arc<Shared>, hooks: Hooks) {
    let id = {
        let mut p = shared.lock();
        if p.clock.is_some() {
            return;
        }
        p.clocks += 1;
        p.clock = Some(p.clocks);
        p.clocks
    };
    let sh = Arc::clone(shared);
    let spawned = thread::Builder::new().name("kota-clock".into()).spawn(move || {
        loop {
            (hooks.sleep)(Duration::from_secs(CLOCK_SECS));
            let p = sh.lock();
            if p.clock != Some(id) {
                return;
            }
            let asleep = p.asleep;
            drop(p);
            if !asleep {
                (hooks.post)();
            }
        }
    });
    if spawned.is_err() {
        stop_clock(shared);
    }
}

/// Retire the minute clock (the item is hidden).
pub fn stop_clock(shared: &Shared) {
    shared.lock().clock = None;
}

/// The main thread's step on `ModuleChanged` (and at start): start a round when one is
/// due, else make sure the timer is armed for when it is. `poll_secs` 0: nothing.
pub fn tick(shared: &Arc<Shared>, settings: &Settings, hooks: Hooks) {
    let now = (hooks.now)();
    let wait = {
        let p = shared.lock();
        if p.asleep || p.running {
            return;
        }
        let fast = p.fast_until.is_some_and(|t| now < t);
        let Some(every) = presence::next_in(&p.presence, settings.poll_secs, fast) else { return };
        let due = p.ended_at.map_or(now, |end| end.saturating_add(every)).max(p.not_before.unwrap_or(0));
        if due > now && p.timer_due == Some(due) {
            return;
        }
        due.saturating_sub(now)
    };
    if wait == 0 {
        round(shared, settings, hooks);
    } else {
        arm(shared, wait, hooks);
    }
}

/// Sleep or lock: drop the round in flight, retire the timer, mark the presence stale.
pub fn suspend(shared: &Shared) {
    shared.stop();
    let mut p = shared.lock();
    p.asleep = true;
    p.presence.mark_stale();
}

/// Wake or unlock: a round after `WAKE_DELAY` (through `tick`, so `poll_secs` 0 stays on
/// demand), with the wake grace for failures.
pub fn resume(shared: &Arc<Shared>, settings: &Settings, hooks: Hooks) {
    let now = (hooks.now)();
    {
        let mut p = shared.lock();
        p.asleep = false;
        p.woke_at = Some(now);
        p.not_before = Some(now + WAKE_DELAY);
        // The round after the wake is due at once, whenever the last one ran.
        p.ended_at = None;
    }
    tick(shared, settings, hooks);
}

/// `kota refresh`: a round now, at most one per `REFRESH_EVERY` seconds.
pub fn refresh(shared: &Arc<Shared>, settings: &Settings, hooks: Hooks) -> String {
    let now = (hooks.now)();
    let (running, started) = {
        let p = shared.lock();
        (p.running, p.started_at)
    };
    if running {
        return "kota: a check is running".into();
    }
    if let Some(ago) = started.map(|t| now.saturating_sub(t)).filter(|ago| *ago < REFRESH_EVERY) {
        return format!("kota: checked {ago} s ago; refresh again in {} s", REFRESH_EVERY - ago);
    }
    round(shared, settings, hooks);
    "kota: checking".into()
}

#[cfg(test)]
mod tests;
