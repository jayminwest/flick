//! What `activity` reads from and shows on the system: the clock, app identity, window
//! titles (`platform::axwatch`), browser tab URLs (`urls::ask`), the menu bar indicator and
//! the quit hook. `Env::default()`
//! is the real thing; tests swap in plain functions. Nothing here runs until the module calls
//! it, and the module calls the title and indicator parts only while recording.

use std::cell::Cell;
use std::path::Path;
use std::sync::Once;

use crate::core::Event;
use crate::platform::status_item::{self, Entry};
use crate::platform::{app, ax, axwatch, clock, events, workspace};
use crate::core::store::{self, Store};

use super::store::Spans;
use super::urls::{self, Ask};

/// What the module reads from the system and changes on it.
pub struct Env {
    pub now: fn() -> i64,
    pub utc_offset: fn(i64) -> i32,
    /// Bundle id (if any) and display name of app `pid`.
    pub identity: fn(i32) -> Option<(Option<String>, String)>,
    pub frontmost: fn() -> Option<i32>,
    /// Flick's own pid: its activations never open spans.
    pub own_pid: i32,
    /// The title of app `pid`'s focused window.
    pub title: fn(i32) -> Option<String>,
    /// Post `Event::WindowChanged` when app `pid`'s focused window or its title changes;
    /// replaces an earlier follow.
    pub follow: fn(i32),
    /// Stop following.
    pub unfollow: fn(),
    /// Flick has Accessibility permission (titles need it).
    pub trusted: fn() -> bool,
    /// Read a browser's front tab URL off the main thread; the answer lands in `Ask::inbox`,
    /// followed by `ModuleChanged`. Never sends an Apple Event in tests.
    pub ask_url: fn(Ask),
    /// Show (true) or hide the menu bar indicator.
    pub indicator: fn(bool),
    /// Close the store's open span when Flick quits. Called each time recording turns on.
    pub on_quit: fn(&Store),
}

impl Default for Env {
    fn default() -> Self {
        Env {
            now: store::now,
            utc_offset: clock::utc_offset,
            identity: workspace::app_identity,
            frontmost: workspace::frontmost_pid,
            own_pid: std::process::id() as i32,
            title: ax::focused_window_title,
            follow: |pid| {
                axwatch::follow(pid, |pid| events::post(Event::WindowChanged { pid }));
            },
            unfollow: axwatch::stop,
            trusted: || ax::is_trusted(false),
            ask_url: urls::ask,
            indicator,
            on_quit,
        }
    }
}

thread_local! {
    /// The indicator's "Stop Recording" was chosen; the module reads it on the event
    /// `request_stop` posts.
    pub static STOP: Cell<bool> = const { Cell::new(false) };
}

/// Take a pending "Stop Recording" from the indicator's menu.
pub fn take_stop() -> bool {
    STOP.take()
}

fn indicator(on: bool) {
    if on {
        let menu = [Entry::Pick { title: "Stop Recording", key: "stop" }];
        status_item::show("activity", "●", "Flick is recording activity", &menu, request_stop);
    } else {
        status_item::hide("activity");
    }
}

/// Menu handler: `AppKit` is mid-event, so ask the module through an event instead of
/// borrowing app state (mulch mx-fcbc43).
fn request_stop(_key: &str) {
    STOP.set(true);
    events::post(Event::ModuleChanged { module: "activity" });
}

/// Hook quit (menu, `quit`, SIGTERM) once to end the open span. The controller may hold the
/// store when quit runs, so the hook opens its own connection to the same file.
fn on_quit(store: &Store) {
    static HOOKED: Once = Once::new();
    let Some(db) = store.conn().path().filter(|p| !p.is_empty()).map(str::to_owned) else {
        return;
    };
    HOOKED.call_once(move || {
        app::on_terminate(move || {
            if let Ok(s) = Store::open(Path::new(&db)) {
                s.close_open(store::now());
            }
        });
        app::quit_on_sigterm();
    });
}
