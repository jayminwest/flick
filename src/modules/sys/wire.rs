//! The real `io::Hooks`: posting `ModuleChanged`, the clock, child processes, connects, the
//! uid, the launcher panel and the peer client the `modules!` line passes in; and the real
//! `window::Hooks`, the "fleet" surface. Tests use `testkit::HOOKS` and `testkit::WINDOW`,
//! so no test runs curl, launchctl, ssh or the probe, asks a peer or opens a window.

use std::sync::{Mutex, PoisonError};

use super::io::Hooks;
use super::window::{self, Note};
use super::{ID, run, unix_now};
use crate::core::Event;
use crate::core::control::PeerHooks;
use crate::platform::surface::{self, Handlers, Input, Keystroke, Size, Spec, SurfaceId};
use crate::platform::{events, panel, workspace};

pub fn hooks(peer: PeerHooks) -> Hooks {
    Hooks {
        post: || events::post(Event::ModuleChanged { module: ID }),
        now: unix_now,
        run: run::run,
        connect: run::connect,
        uid: run::uid,
        run_input: run::run_input,
        ask: peer.ask,
        visible: panel::is_visible,
        open: workspace::open_url,
    }
}

const SURFACE: SurfaceId = SurfaceId("fleet");

/// What the window's handlers queued, taken by the module on `ModuleChanged`.
static NOTES: Mutex<Vec<Note>> = Mutex::new(Vec::new());

fn queue(note: Note) {
    NOTES.lock().unwrap_or_else(PoisonError::into_inner).push(note);
    events::post(Event::ModuleChanged { module: ID });
}

fn key(_: SurfaceId, k: Keystroke) -> bool {
    window::note(k, || surface::input(SURFACE)).map(queue).is_some()
}

fn open() {
    let spec = Spec {
        title: "Fleet",
        autosave: "FlickFleet",
        size: Size { w: 420.0, h: 520.0 },
        min_size: Size { w: 320.0, h: 260.0 },
        placeholder: "Filter machines and services (Return)",
        input: Input::Single,
        hide_on_blur: false,
        private: None,
        align: surface::Align::Top,
    };
    let handlers = Handlers {
        // Return is taken by `key`; nothing else submits.
        submit: |_, _| {},
        key,
        chip_removed: |_, _| {},
        // Hidden: the next tick polls no more (`Sys::poll`).
        closed: |_| {},
        card_action: |_, card, action, _| queue(Note::Press { card: card.into(), action: action.into() }),
    };
    surface::open(SURFACE, &spec, handlers);
}

pub const WINDOW: window::Hooks = window::Hooks {
    open,
    show: || surface::show(SURFACE),
    hide: || surface::hide(SURFACE),
    visible: || surface::is_visible(SURFACE),
    key: || surface::is_key(SURFACE),
    header: |title, subtitle, status| surface::set_header(SURFACE, &surface::Header { title, subtitle, status }),
    rows: |rows| surface::set_rows(SURFACE, rows),
    notice: |notice| surface::set_notice(SURFACE, notice),
    take: || std::mem::take(&mut *NOTES.lock().unwrap_or_else(PoisonError::into_inner)),
    snapshot: |path| surface::snapshot(SURFACE, path),
};
