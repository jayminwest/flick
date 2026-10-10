//! The real `io::Hooks`: posting `ModuleChanged`, the clock, child processes, the host
//! name and sleeping. Tests use `testkit::HOOKS`, so no test runs herdr or curl.

use std::thread;

use super::io::Hooks;
use super::{ID, run, unix_now};
use crate::core::Event;
use crate::platform::{clock, events};

pub const HOOKS: Hooks = Hooks {
    post: || events::post(Event::ModuleChanged { module: ID }),
    now: unix_now,
    run: run::run,
    host: run::host,
    sleep: thread::sleep,
    utc_offset: clock::utc_offset,
};
