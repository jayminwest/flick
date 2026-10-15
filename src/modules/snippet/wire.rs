//! Types a snippet: after a short settle, so the launcher has given focus back to the
//! previous app, and once the modifiers of the hotkey or chord that asked for it are up (at
//! most `RELEASE_CAP`), so the app never reads ⌘ or ⌃ with the text.

use std::time::{Duration, Instant};

use crate::core::store;
use crate::platform::{ax, clock, keytap, timer};

/// Shift, control, option and command (`CGEventFlags`).
const MODIFIERS: u64 = 0x1e_0000;
/// Seconds before the first try, for focus to return to the previous app.
const SETTLE: f64 = 0.08;
/// Seconds between modifier checks while waiting.
const POLL: f64 = 0.02;
/// The longest typing waits for the modifiers to come up.
const RELEASE_CAP: Duration = Duration::from_secs(1);

pub fn now() -> (i64, i32) {
    let ts = store::now();
    (ts, clock::utc_offset(ts))
}

pub fn type_text(text: String) {
    if !ax::ensure_trusted() {
        return;
    }
    let until = Instant::now() + RELEASE_CAP;
    timer::after(SETTLE, move || wait(text.clone(), until));
}

fn wait(text: String, until: Instant) {
    if keytap::current_flags() & MODIFIERS != 0 && Instant::now() < until {
        timer::after(POLL, move || wait(text.clone(), until));
    } else {
        keytap::type_text(&text);
    }
}
