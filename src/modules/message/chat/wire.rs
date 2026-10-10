//! The real chat `Hooks`: the "chat" surface, the front app, Accessibility and ssh. The
//! surface's handlers only queue a `Note` and post `ModuleChanged` (mx-fcbc43); `key` also
//! answers at once whether the chat takes the keystroke. Tests use fakes, so no test opens
//! a window or runs ssh.

use std::sync::{Mutex, PoisonError};

use super::model;
use super::session::{Hooks, Note, binding};
use crate::core::Event;
use crate::modules::message::run;
use crate::platform::surface::{self, Handlers, Input, Keystroke, Size, Spec, SurfaceId};
use crate::platform::{ax, events, workspace};

const ID: SurfaceId = SurfaceId("chat");

/// What the handlers queued, taken by the module on `ModuleChanged`.
static NOTES: Mutex<Vec<Note>> = Mutex::new(Vec::new());

fn queue(note: Note) {
    NOTES.lock().unwrap_or_else(PoisonError::into_inner).push(note);
    events::post(Event::ModuleChanged { module: "message" });
}

fn take() -> Vec<Note> {
    std::mem::take(&mut *NOTES.lock().unwrap_or_else(PoisonError::into_inner))
}

fn key(_: SurfaceId, k: Keystroke) -> bool {
    binding(k).map(|c| queue(Note::Key(c))).is_some()
}

fn open() {
    let spec = Spec {
        title: "KOTA",
        autosave: "FlickChat",
        size: Size { w: 460.0, h: 560.0 },
        min_size: Size { w: 340.0, h: 300.0 },
        placeholder: "Ask KOTA…",
        input: Input::Multi,
        hide_on_blur: false,
        private: None,
    };
    let handlers = Handlers {
        submit: |_, text| queue(Note::Submit(text)),
        key,
        // Context chips come with flick-65bd.
        chip_removed: |_, _| {},
        closed: |_| queue(Note::Closed),
        card_action: |_, card, action, values| {
            queue(Note::Press { card: card.into(), action: action.into(), values });
        },
    };
    surface::open(ID, &spec, handlers);
}

pub const HOOKS: Hooks = Hooks {
    open,
    show: || surface::show(ID),
    hide: || surface::hide(ID),
    visible: || surface::is_visible(ID),
    key: || surface::is_key(ID),
    header: |title, subtitle, status| surface::set_header(ID, &surface::Header { title, subtitle, status }),
    rows: |rows| surface::set_rows(ID, rows),
    notice: |notice| surface::set_notice(ID, notice),
    set_input: |text| surface::set_input(ID, text),
    take,
    front: workspace::frontmost_pid,
    refocus: |pid| {
        let _ = ax::make_frontmost(pid);
    },
    ask: run::exec,
    new_thread: model::new_thread_id,
    snapshot: |path| surface::snapshot(ID, path),
};
