//! The real `Hooks`: `platform::winman` and its `hide`, the quit hook and the log. Tests use
//! fakes, so no test watches an app or moves a window.

use std::path::Path;
use std::sync::Once;

use super::{ID, QUIT_WAIT, put_back};
use crate::core::Event;
use crate::core::store::Store;
use crate::platform::winman::hide::{self, Batch};
use crate::platform::winman::{self, WinId, WinInfo};
use crate::platform::{Rect, app, events};

/// What the module reads from and does to the windows on screen.
#[derive(Clone, Copy)]
pub struct Hooks {
    /// Start watching every app's windows (`winman::start`); false without Accessibility
    /// permission. Window changes post `ModuleChanged`.
    pub start: fn() -> bool,
    pub stop: fn(),
    /// Drop winman's queued changes: the module reads the whole snapshot each time.
    pub drain: fn(),
    pub snapshot: fn() -> Vec<WinInfo>,
    /// Move windows to frames; the batch is done once every app applied them.
    pub place: fn(&[(WinId, Rect)]) -> Batch,
    /// Park windows (id, frame now) at their screen's hide corner.
    pub hide: fn(&[(WinId, Rect)]) -> Batch,
    /// The window server still has the window.
    pub exists: fn(WinId) -> bool,
    /// Where to put a window that sits parked at a screen corner; None for any other.
    pub stray: fn(Rect) -> Option<Rect>,
    /// Put back every window in this store's `tile_hidden` when Flick quits (menu, `quit`,
    /// SIGTERM). Called at `Started`; hooks once.
    pub on_quit: fn(&Store),
    /// Put back every window in `tile_hidden` of the flick.db at this path, now.
    pub put_back: fn(&str),
    pub log: fn(&str),
}

pub const HOOKS: Hooks = Hooks {
    start: || winman::start(changed),
    stop: winman::stop,
    drain: || {
        winman::drain();
    },
    snapshot: winman::snapshot,
    place: |frames| hide::place(frames, changed),
    hide: |windows| hide::hide(windows, changed),
    exists: hide::exists,
    stray: hide::stray,
    on_quit,
    put_back: put_back_at,
    log: |line| eprintln!("flick: {ID}: {line}"),
};

fn changed() {
    events::post(Event::ModuleChanged { module: ID });
}

/// The controller may hold the store and the module when quit runs, so the hook opens its own
/// connection to the same file and touches no module state (mulch mx-3f26b9).
fn on_quit(store: &Store) {
    static HOOKED: Once = Once::new();
    let Some(db) = store.conn().path().filter(|p| !p.is_empty()).map(str::to_owned) else {
        return;
    };
    HOOKED.call_once(move || {
        app::on_terminate(move || put_back_at(&db));
        app::quit_on_sigterm();
    });
}

fn put_back_at(db: &str) {
    if let Ok(store) = Store::open(Path::new(db)) {
        put_back(&store, &HOOKS, QUIT_WAIT);
    }
}
