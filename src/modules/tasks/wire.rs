//! What `task` reads from and does to the system: the clock, posting `Event::TaskChanged`
//! and the quit hook. `Env::default()` is the real thing; tests swap in plain functions.

use std::path::Path;
use std::sync::Once;

use crate::core::Event;
use crate::platform::{app, clock, events};
use crate::core::store::{self, Store};

use super::store::TaskStore;

pub struct Env {
    pub now: fn() -> i64,
    pub utc_offset: fn(i64) -> i32,
    /// Queue an event for every module (`events::post`): delivered on the next main-queue
    /// turn, so it never runs inside the current dispatch.
    pub post: fn(Event),
    /// End the store's open time row when Flick quits. Called each time a task starts.
    pub on_quit: fn(&Store),
}

impl Default for Env {
    fn default() -> Self {
        Env { now: store::now, utc_offset: clock::utc_offset, post: events::post, on_quit }
    }
}

/// Hook quit (menu, `quit`, SIGTERM) once to end the open time row. The controller may hold
/// the store and the module when quit runs, so the hook opens its own connection to the
/// same file and touches no module state (mulch mx-3f26b9).
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
