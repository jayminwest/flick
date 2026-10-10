//! The real hooks: `io::Hooks` (curl through `std::process`, `ModuleChanged` through
//! `platform::events`), `chat::Ui` (the "llm" surface, the front app) and
//! `private_chat::Ui` (the private "llm-private" surface, the concealed copy, the quit hook).
//! Tests use `testkit::HOOKS`, `testkit::UI` and `testkit::PRIVATE`, so no test runs curl or
//! opens a window. The surfaces' handlers only queue a `Note` (one queue per surface) and post
//! `ModuleChanged` (mx-fcbc43); `key` also answers at once whether the chat takes the
//! keystroke.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Once, PoisonError, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use super::ID;
use super::chat::{Note, Ui, binding, thread_id};
use super::io::{Hooks, Shared};
use super::private_chat::{self, BANNER, Room, TITLE};
use super::transport::{Child, Spawned};
use crate::core::Event;
use crate::platform::surface::{self, Handlers, Input, Keystroke, Size, Spec, SurfaceId};
use crate::platform::{app, ax, events, pasteboard, workspace};

pub const HOOKS: Hooks = Hooks { spawn, post, sleep: std::thread::sleep };

fn post() {
    events::post(Event::ModuleChanged { module: ID });
}

impl Child for std::process::Child {
    fn kill(&mut self) {
        let _ = std::process::Child::kill(self);
    }

    fn wait(&mut self) -> Option<i32> {
        std::process::Child::wait(self).ok().and_then(|s| s.code())
    }
}

/// Start `argv` (always `/usr/bin/curl`, `-q` first) with all three pipes.
fn spawn(argv: &[String]) -> Result<Spawned, String> {
    let (program, rest) = argv.split_first().ok_or("empty command")?;
    let mut child = Command::new(program)
        .args(rest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = std::process::Child::kill(&mut child);
        return Err(format!("{program}: no pipes"));
    };
    Ok(Spawned { stdin: Box::new(stdin), stdout: Box::new(stdout), stderr: Box::new(stderr), child: Box::new(child) })
}

const SURFACE: SurfaceId = SurfaceId("llm");

/// What the handlers queued, taken by the module on `ModuleChanged`.
static NOTES: Mutex<Vec<Note>> = Mutex::new(Vec::new());

fn push(notes: &Mutex<Vec<Note>>, note: Note) {
    notes.lock().unwrap_or_else(PoisonError::into_inner).push(note);
    post();
}

fn drain(notes: &Mutex<Vec<Note>>) -> Vec<Note> {
    std::mem::take(&mut *notes.lock().unwrap_or_else(PoisonError::into_inner))
}

fn queue(note: Note) {
    push(&NOTES, note);
}

fn take() -> Vec<Note> {
    drain(&NOTES)
}

fn key(_: SurfaceId, k: Keystroke) -> bool {
    binding(k).map(|c| queue(Note::Key(c))).is_some()
}

fn open() {
    let spec = Spec {
        title: "Local Model",
        autosave: "FlickLlmChat",
        size: Size { w: 520.0, h: 620.0 },
        min_size: Size { w: 340.0, h: 300.0 },
        placeholder: "Message the model…",
        input: Input::Multi,
        hide_on_blur: false,
        private: None,
        align: surface::Align::Bottom,
    };
    let handlers = Handlers {
        submit: |_, text| queue(Note::Submit(text)),
        key,
        chip_removed: |_, _| {},
        closed: |_| queue(Note::Closed),
        card_action: |_, _, _, _| {},
    };
    surface::open(SURFACE, &spec, handlers);
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn new_id() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    thread_id(ms, N.fetch_add(1, Ordering::Relaxed) % 100)
}

pub const UI: Ui = Ui {
    open,
    show: || surface::show(SURFACE),
    hide: || surface::hide(SURFACE),
    visible: || surface::is_visible(SURFACE),
    key: || surface::is_key(SURFACE),
    header: |title, subtitle, status| surface::set_header(SURFACE, &surface::Header { title, subtitle, status }),
    rows: |rows| surface::set_rows(SURFACE, rows),
    notice: |notice| surface::set_notice(SURFACE, notice),
    set_input: |text| surface::set_input(SURFACE, text),
    take,
    front: workspace::frontmost_pid,
    refocus: |pid| {
        let _ = ax::make_frontmost(pid);
    },
    now,
    new_id,
};

const PRIVATE: SurfaceId = SurfaceId("llm-private");

/// The private window's notes, apart from the normal window's.
static PRIVATE_NOTES: Mutex<Vec<Note>> = Mutex::new(Vec::new());

fn private_queue(note: Note) {
    push(&PRIVATE_NOTES, note);
}

fn private_key(_: SurfaceId, k: Keystroke) -> bool {
    private_chat::binding(k).map(|c| private_queue(Note::Key(c))).is_some()
}

fn private_open() {
    let spec = Spec {
        title: TITLE,
        autosave: "FlickLlmPrivate",
        size: Size { w: 520.0, h: 620.0 },
        min_size: Size { w: 340.0, h: 300.0 },
        placeholder: "Message the model privately…",
        input: Input::Multi,
        hide_on_blur: false,
        private: Some(BANNER),
        align: surface::Align::Bottom,
    };
    let handlers = Handlers {
        submit: |_, text| private_queue(Note::Submit(text)),
        key: private_key,
        chip_removed: |_, _| {},
        closed: |_| private_queue(Note::Closed),
        // The private chat shows no cards.
        card_action: |_, _, _, _| {},
    };
    surface::open(PRIVATE, &spec, handlers);
}

/// The running module's room and calls, for the quit hook (weak: a dropped module wipes on
/// its own).
static HELD: Mutex<Option<(Weak<Room>, Weak<Shared>)>> = Mutex::new(None);

/// Point the quit hook at `room` and `shared`; the first time, install it (menu, `quit`,
/// SIGTERM). It touches only `HELD`, never app state (mx-3f26b9).
fn hook_quit(room: &Arc<Room>, shared: &Arc<Shared>) {
    static HOOKED: Once = Once::new();
    *HELD.lock().unwrap_or_else(PoisonError::into_inner) = Some((Arc::downgrade(room), Arc::downgrade(shared)));
    HOOKED.call_once(|| {
        app::on_terminate(|| {
            let held = HELD.lock().unwrap_or_else(PoisonError::into_inner).take();
            if let Some((room, shared)) = held {
                private_chat::wipe_for_quit(&room, &shared);
            }
        });
        app::quit_on_sigterm();
    });
}

pub const PRIVATE_UI: private_chat::Ui = private_chat::Ui {
    win: Ui {
        open: private_open,
        show: || surface::show(PRIVATE),
        hide: || surface::hide(PRIVATE),
        visible: || surface::is_visible(PRIVATE),
        key: || surface::is_key(PRIVATE),
        header: |title, subtitle, status| surface::set_header(PRIVATE, &surface::Header { title, subtitle, status }),
        rows: |rows| surface::set_rows(PRIVATE, rows),
        notice: |notice| surface::set_notice(PRIVATE, notice),
        set_input: |text| surface::set_input(PRIVATE, text),
        take: || drain(&PRIVATE_NOTES),
        ..UI
    },
    copy: pasteboard::set_text_concealed,
    hook_quit,
};
