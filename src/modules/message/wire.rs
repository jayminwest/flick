//! What `message` does to the system: the clock, the HUD, notifications, the pasteboard,
//! opening links and apps, the KOTA action command, local runs (`local.rs`), the path of
//! this binary and the HUD's press and dismiss handlers.
//! `Env::default()` is the real thing; tests swap in plain functions and never register the
//! real handlers (mx-444675).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use std::path::PathBuf;

use super::dispatch::Note;
use super::local::{self, Ran};
use super::run::{self, Exit, Job};
use crate::core::Event;
use crate::core::card::Card;
use crate::core::store;
use crate::platform::hud::{self, CardUi, Content, Options, Placement};
use crate::platform::{clock, events, notify, pasteboard, timer, workspace};

pub struct Env {
    pub now: fn() -> i64,
    pub utc_offset: fn(i64) -> i32,
    /// A fresh message id.
    pub new_id: fn() -> String,
    /// Show or redraw the card with this id; true when it is new.
    pub show: fn(&str, &Content, &Placement, &Options) -> bool,
    /// Remove one card (a pending post its reply replaces); true when it showed.
    pub dismiss: fn(&str) -> bool,
    /// Remove every card.
    pub hide: fn(),
    /// (id, title, body) as a system notification.
    pub notify: fn(&str, &str, &str),
    pub copy: fn(&str),
    pub open_url: fn(&str),
    /// Open an app by name or bundle id; `Err` when none matches.
    pub open_app: fn(&str) -> Result<(), String>,
    /// This binary, which `script` and `flick` actions re-run as a client.
    pub self_exe: fn() -> Result<PathBuf, String>,
    /// Run one local process (on the worker thread).
    pub run_local: fn(&local::Job) -> Ran,
    /// Show or redraw a structured card with its press state; true when it is new.
    pub show_card: fn(&Card, &CardUi, &Placement, &Options) -> bool,
    /// Redraw a structured card if it shows; true when it does.
    pub update_card: fn(&Card, &CardUi, &Options) -> bool,
    /// Move the keyboard into the newest card without activating Flick; false when none
    /// shows.
    pub focus: fn() -> bool,
    /// Give the keyboard back if a card holds it; true when one did.
    pub unfocus: fn() -> bool,
    /// Register the HUD's press and dismiss handlers (once; later calls do nothing).
    pub subscribe: fn(),
    /// Take what the HUD handlers queued.
    pub take_notes: fn() -> Vec<Note>,
    /// Run one KOTA send (on the worker thread).
    pub exec: fn(&Job) -> Exit,
    /// Tell the module, from any thread, that work finished (`ModuleChanged`).
    pub changed: fn(),
    /// Post `ModuleChanged` after this many seconds (the pending watchdog).
    pub wake_after: fn(u64),
    /// Tell every module how many cards wait on the user (`Event::CardsPending`).
    pub pending: fn(u32),
}

impl Default for Env {
    fn default() -> Self {
        Env {
            now: store::now,
            utc_offset: clock::utc_offset,
            new_id,
            show: hud::show,
            dismiss: hud::dismiss,
            hide: hud::dismiss_all,
            notify: |id, title, body| {
                if let Err(e) = notify::post(&format!("message:{id}"), title, body) {
                    eprintln!("flick: message: notification: {e}");
                }
            },
            copy: pasteboard::set_text,
            open_url: workspace::open_url,
            open_app,
            self_exe: || std::env::current_exe().map_err(|e| format!("cannot find Flick's binary: {e}")),
            run_local: local::exec,
            show_card: hud::show_card,
            update_card: hud::update_card,
            focus: hud::focus_top,
            unfocus: hud::unfocus,
            subscribe,
            take_notes,
            exec: run::exec,
            changed,
            wake_after: |secs| timer::after(secs as f64, changed),
            pending: |count| events::post(Event::CardsPending { count }),
        }
    }
}

/// What the HUD handlers queued, drained by the module on `ModuleChanged`.
static NOTES: Mutex<Vec<Note>> = Mutex::new(Vec::new());

fn changed() {
    events::post(Event::ModuleChanged { module: "message" });
}

/// Queue `note` and wake the module. Runs inside `AppKit` callbacks, so it never touches app
/// state (mx-fcbc43).
fn queue(note: Note) {
    NOTES.lock().unwrap_or_else(PoisonError::into_inner).push(note);
    changed();
}

fn take_notes() -> Vec<Note> {
    std::mem::take(&mut *NOTES.lock().unwrap_or_else(PoisonError::into_inner))
}

fn open_app(app: &str) -> Result<(), String> {
    let path = workspace::find_app(app).ok_or_else(|| format!("open_app: no app {app:?}"))?;
    workspace::open_file(&path);
    Ok(())
}

fn subscribe() {
    hud::on_press(|card, action, values| {
        queue(Note::Press { card: card.into(), action: action.into(), values });
    });
    hud::on_dismiss(|card, why| {
        queue(match why {
            hud::Dismissed::Timeout => Note::Expired(card.into()),
            _ => Note::Dismissed(card.into()),
        });
    });
}

/// `m` and the time in milliseconds, base 36, plus a counter so two posts in one
/// millisecond differ.
fn new_id() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let n = N.fetch_add(1, Ordering::Relaxed) % 36;
    format!("m{}{}", base36(ms), base36(u128::from(n)))
}

fn base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = vec![];
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_short_and_distinct() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(36 * 36 + 35), "10z");
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert!(a.starts_with('m') && crate::core::card::valid_id(&a), "{a}");
    }
}
