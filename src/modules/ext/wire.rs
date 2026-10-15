//! The real hooks: post `ModuleChanged`, log to stderr, post a notification.

use crate::core::Event;
use crate::platform::{events, notify};

/// Worker thread: the inbox has an answer.
pub fn post() {
    events::post(Event::ModuleChanged { module: "ext" });
}

pub fn log(line: &str) {
    eprintln!("flick: ext: {line}");
}

/// Main thread: a banner for a failed call of `ext` (logged already).
pub fn notify(ext: &str, error: &str) {
    let _ = notify::post(&format!("ext:{ext}"), "Flick extension failed", error);
}
