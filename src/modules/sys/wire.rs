//! The real `io::Hooks`: posting `ModuleChanged`, the clock and child processes. Tests use
//! `testkit::HOOKS`, so no test runs the probe.

use super::io::Hooks;
use super::{ID, run, unix_now};
use crate::core::Event;
use crate::platform::events;

pub const HOOKS: Hooks = Hooks {
    post: || events::post(Event::ModuleChanged { module: ID }),
    now: unix_now,
    run: run::run,
};
