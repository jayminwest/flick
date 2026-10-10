//! The real `io::Hooks`: posting `ModuleChanged`, the clock, child processes, this
//! binary's path, the host name and sleeping. Tests use `testkit::HOOKS`, so no test runs
//! herdr, curl, ssh or Flick.

use std::thread;

use super::io::Hooks;
use super::{ID, run, unix_now};
use crate::core::Event;
use crate::platform::{clock, events};

pub const HOOKS: Hooks = Hooks {
    post: || events::post(Event::ModuleChanged { module: ID }),
    now: unix_now,
    run: run::run,
    feed: run::feed,
    exe: run::exe,
    host: run::host,
    sleep: thread::sleep,
    utc_offset: clock::utc_offset,
};
