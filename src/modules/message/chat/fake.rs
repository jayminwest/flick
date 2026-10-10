//! Fake chat hooks for the module's tests: a window that is only state in a `thread_local`,
//! and an ssh that never runs. The window's calls go to the module tests' log (`chat …`).

use std::cell::{Cell, RefCell};

use super::session::{Hooks, Note};
use crate::modules::message::run::{Exit, Job};
use crate::modules::message::tests::log;
use crate::platform::surface::{self, Row};

thread_local! {
    static VISIBLE: Cell<bool> = const { Cell::new(false) };
    static KEY: Cell<bool> = const { Cell::new(false) };
    static FRONT: Cell<Option<i32>> = const { Cell::new(Some(42)) };
    static NOTES: RefCell<Vec<Note>> = const { RefCell::new(Vec::new()) };
    static THREADS: Cell<u32> = const { Cell::new(0) };
}

/// Queue `note` as the window's handlers would.
pub fn queue(note: Note) {
    NOTES.with(|n| n.borrow_mut().push(note));
}

/// Make another app (`pid`) the one in front, or none.
pub fn set_front(pid: Option<i32>) {
    FRONT.set(pid);
}

/// Esc: the surface hid itself and queued `Closed`.
pub fn closed() {
    VISIBLE.set(false);
    KEY.set(false);
    queue(Note::Closed);
}

/// Every question `order@host` was asked, in order (any thread).
pub fn asked() -> Vec<String> {
    ORDER.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

static ORDER: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Take the keyboard from the window (the user clicked another app), keeping it shown.
pub fn blur() {
    KEY.set(false);
}

pub fn visible() -> bool {
    VISIBLE.get()
}

/// One row as a line: `kind key state|side|header|md`.
fn row(r: &Row) -> String {
    match *r {
        Row::Bubble { key, side, header, md, state, .. } => format!("{key} {state:?}|{side:?}|{header}|{md}"),
        Row::Card { key, ui, .. } => format!("{key} card|{}|{:?}", ui.pending, ui.error),
        Row::Divider { text } => format!("-- {text}"),
    }
}

/// A fake ssh, run on the worker thread: the host picks the exit; `echo` fails with the
/// argv and stdin, so tests read them off the notice.
fn ask(job: &Job) -> Exit {
    match job.argv[5].as_str() {
        "ok@host" => Exit::Sent,
        "order@host" => {
            ORDER.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(job.stdin.clone());
            Exit::Sent
        }
        "reject@host" => Exit::Rejected("not now".into()),
        "echo@host" => Exit::Failed(format!("{} <{}", job.argv[6..].join(" "), job.stdin)),
        other => Exit::Failed(format!("ssh {other}: down")),
    }
}

pub const HOOKS: Hooks = Hooks {
    open: || log("chat open".into()),
    show: || {
        VISIBLE.set(true);
        KEY.set(true);
        log("chat show".into());
    },
    hide: || {
        VISIBLE.set(false);
        KEY.set(false);
        log("chat hide".into());
    },
    visible,
    key: || KEY.get(),
    header: |title, subtitle, status| {
        let status = match status {
            surface::Status::Idle => "idle",
            surface::Status::Busy => "busy",
            surface::Status::Error => "error",
        };
        log(format!("chat header {title}|{subtitle}|{status}"));
    },
    rows: |rows| log(format!("chat rows {}", rows.iter().map(row).collect::<Vec<_>>().join(" / "))),
    notice: |n| {
        if let Some(n) = n {
            log(format!("chat notice {n}"));
        }
    },
    set_input: |t| log(format!("chat input {t}")),
    take: || NOTES.with(|n| std::mem::take(&mut *n.borrow_mut())),
    front: || FRONT.get(),
    refocus: |pid| log(format!("chat refocus {pid}")),
    ask,
    new_thread: || {
        let n = THREADS.get() + 1;
        THREADS.set(n);
        format!("tnew{n}")
    },
    snapshot: |path| {
        log(format!("chat snapshot {path}"));
        if path.starts_with('/') { Ok(()) } else { Err(format!("can't write {path}")) }
    },
};
