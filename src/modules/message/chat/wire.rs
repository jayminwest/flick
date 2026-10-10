//! The real chat `Hooks`: the "chat" surface, the front app, Accessibility, the clipboard,
//! screenshots and ssh. The
//! surface's handlers only queue a `Note` and post `ModuleChanged` (mx-fcbc43); `key` also
//! answers at once whether the chat takes the keystroke. Tests use fakes, so no test opens
//! a window or runs ssh.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::{attach, model};
use super::session::{Hooks, Note, binding};
use crate::core::Event;
use crate::modules::message::run;
use crate::platform::surface::{self, Handlers, Input, Keystroke, Size, Spec, SurfaceId};
use crate::platform::capture::{self, Shot};
use crate::platform::context::{self, rules::ShotError};
use crate::platform::{ax, events, pasteboard, workspace};

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
        chip_removed: |_, n| queue(Note::Unchip(n)),
        closed: |_| queue(Note::Closed),
        card_action: |_, card, action, values| {
            queue(Note::Press { card: card.into(), action: action.into(), values });
        },
    };
    surface::open(ID, &spec, handlers);
}

/// ⌘⇧S: screenshot the display under the pointer with the window hidden, then queue the PNG.
fn shoot() {
    context::shoot_display(|| surface::hide(ID), || surface::show(ID), |shot| queue(Note::Shot(png(shot))));
}

/// The shot's bytes, its temp file deleted whatever happens (on the shot's worker thread).
fn png(shot: Result<Shot, ShotError>) -> Result<Arc<[u8]>, String> {
    match shot {
        Ok(shot) => {
            let bytes = std::fs::read(&shot.path);
            let _ = std::fs::remove_file(&shot.path);
            bytes.map(Arc::from).map_err(|e| format!("screenshot: {e}"))
        }
        Err(ShotError::NotPermitted) => {
            // Once per run: the first ask shows macOS's prompt, later ones only answer.
            static ASKED: AtomicBool = AtomicBool::new(false);
            if !ASKED.swap(true, Ordering::Relaxed) {
                events::on_main(|| {
                    capture::request_permission();
                });
            }
            Err(ShotError::NotPermitted.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
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
    context: || context::front(true),
    clipboard: pasteboard::copied_text,
    copy: pasteboard::set_text,
    shoot,
    chips: |chips| surface::set_chips(ID, chips),
    upload: |argv, png| run::exec_bytes(argv, png.to_vec(), attach::BUDGET),
};
