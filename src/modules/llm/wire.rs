//! The real hooks: `io::Hooks` (curl through `std::process`, `ModuleChanged` through
//! `platform::events`) and `chat::Ui` (the "llm" surface, the front app). Tests use
//! `testkit::HOOKS` and `testkit::UI`, so no test runs curl or opens a window. The surface's
//! handlers only queue a `Note` and post `ModuleChanged` (mx-fcbc43); `key` also answers at
//! once whether the chat takes the keystroke.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use super::ID;
use super::chat::{Note, Ui, binding, thread_id};
use super::io::Hooks;
use super::transport::{Child, Spawned};
use crate::core::Event;
use crate::platform::surface::{self, Handlers, Input, Keystroke, Size, Spec, SurfaceId};
use crate::platform::{ax, events, workspace};

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

fn queue(note: Note) {
    NOTES.lock().unwrap_or_else(PoisonError::into_inner).push(note);
    post();
}

fn take() -> Vec<Note> {
    std::mem::take(&mut *NOTES.lock().unwrap_or_else(PoisonError::into_inner))
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
