//! Fakes for the module's tests: `spawn` answers by URL (the argv's last word) with canned
//! output, so no test runs curl or reaches a server. Every argv and stdin body lands in
//! `SENT` for the assertions that prompt text goes over stdin only. `UI` is a chat window
//! that is only state in `thread_local`s; its calls land in `take_log`. `PRIVATE` is the
//! private window: the same fake, with its own note queue (`queue_private`), and its copies
//! and quit hooks logged.

use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use super::chat::{Note, Ui};
use super::io::Hooks;
use super::private_chat;
use super::transport::{Child, Spawned};
use crate::platform::surface::Row;

pub const STREAM: &str = include_str!("fixtures/chat_stream.txt");
pub const REASONING: &str = include_str!("fixtures/chat_reasoning.txt");
pub const MLX_MODELS: &str = include_str!("fixtures/models_mlx.json");

pub const HOOKS: Hooks = Hooks { spawn, post: || {}, sleep: |d| std::thread::sleep(d / 50) };

/// Hooks whose watchdog never fires within a test.
pub const PATIENT: Hooks = Hooks { sleep: |_| std::thread::sleep(Duration::from_secs(3_600)), ..HOOKS };

/// One spawn: its argv and what was written to its stdin.
#[derive(Clone, Debug, Default)]
pub struct Sent {
    pub argv: Vec<String>,
    pub stdin: Vec<u8>,
}

pub static SENT: Mutex<Vec<Sent>> = Mutex::new(vec![]);

/// What was sent to `url`, in order.
pub fn sent(url: &str) -> Vec<Sent> {
    let sent = SENT.lock().unwrap_or_else(PoisonError::into_inner);
    sent.iter().filter(|s| s.argv.last().is_some_and(|u| u == url)).cloned().collect()
}

/// A killed flag that blocking readers wait on.
#[derive(Default)]
struct Gate {
    killed: Mutex<bool>,
    cond: Condvar,
}

struct FakeChild {
    code: Option<i32>,
    gate: Arc<Gate>,
}

impl Child for FakeChild {
    fn kill(&mut self) {
        self.code = None;
        *self.gate.killed.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.gate.cond.notify_all();
    }

    fn wait(&mut self) -> Option<i32> {
        self.code
    }
}

/// Gives `head`, then blocks until the child is killed (or `head` only, without a gate).
struct Hang {
    head: Cursor<Vec<u8>>,
    gate: Arc<Gate>,
}

impl Read for Hang {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.head.read(buf)?;
        if n > 0 {
            return Ok(n);
        }
        let mut killed = self.gate.killed.lock().unwrap_or_else(PoisonError::into_inner);
        while !*killed {
            killed = self.gate.cond.wait(killed).unwrap_or_else(PoisonError::into_inner);
        }
        Ok(0)
    }
}

/// Records stdin into `SENT[index]`.
struct Stdin(usize);

impl Write for Stdin {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut sent = SENT.lock().unwrap_or_else(PoisonError::into_inner);
        sent[self.0].stdin.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Canned answers by host (`http://<host>/v1/...`):
/// - `mlx`: `MLX_MODELS` / `STREAM`; `reason`: `REASONING`;
/// - `down`: curl's connect error, exit 7; `err`: an error body, exit 22;
/// - `gateway`: a proxy's empty 502 (nothing behind `tailscale serve`), exit 22;
/// - `junk`: a non-JSON body, exit 0; `empty`: nothing, exit 0;
/// - `inband`: an error chunk inside the stream; `none`: an empty model list;
/// - `hang`: blocks until killed; `half`: one text chunk, then blocks until killed;
/// - `nospawn`: curl does not start.
fn spawn(argv: &[String]) -> Result<Spawned, String> {
    let url = argv.last().cloned().unwrap_or_default();
    let index = {
        let mut sent = SENT.lock().unwrap_or_else(PoisonError::into_inner);
        sent.push(Sent { argv: argv.to_vec(), stdin: vec![] });
        sent.len() - 1
    };
    let host = url.trim_start_matches("http://").split(['/', ':']).next().unwrap_or_default();
    let models = url.ends_with("/v1/models");
    let gate = Arc::new(Gate::default());
    let (out, err, code, hang): (&str, &str, Option<i32>, bool) = match host {
        "nospawn" => return Err("/usr/bin/curl: No such file or directory".into()),
        "mlx" if models => (MLX_MODELS, "", Some(0), false),
        "mlx" => (STREAM, "", Some(0), false),
        "reason" => (REASONING, "", Some(0), false),
        "down" => ("", "curl: (7) Failed to connect to down port 80 after 1 ms: Couldn't connect to server\n", Some(7), false),
        "err" => (r#"{"error":{"message":"model not found"}}"#, "curl: (22) The requested URL returned error: 404\n", Some(22), false),
        "junk" => ("<html>hi</html>\n", "", Some(0), false),
        "gateway" => ("", "curl: (22) The requested URL returned error: 502\n", Some(22), false),
        "none" if models => (r#"{"object":"list","data":[]}"#, "", Some(0), false),
        "inband" => ("data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\ndata: {\"error\":\"overloaded\"}\n", "", Some(0), false),
        "hang" => ("", "", Some(0), true),
        "half" => ("data: {\"choices\":[{\"delta\":{\"content\":\"part\"}}]}\n", "", Some(0), true),
        _ => ("", "", Some(0), false),
    };
    let stdout: Box<dyn Read + Send> = if hang {
        Box::new(Hang { head: Cursor::new(out.as_bytes().to_vec()), gate: Arc::clone(&gate) })
    } else {
        Box::new(Cursor::new(out.as_bytes().to_vec()))
    };
    Ok(Spawned {
        stdin: Box::new(Stdin(index)),
        stdout,
        stderr: Box::new(Cursor::new(err.as_bytes().to_vec())),
        child: Box::new(FakeChild { code, gate }),
    })
}

thread_local! {
    static VISIBLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static KEY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FRONT: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(Some(42)) };
    static NOTES: std::cell::RefCell<Vec<Note>> = const { std::cell::RefCell::new(Vec::new()) };
    static PRIVATE_NOTES: std::cell::RefCell<Vec<Note>> = const { std::cell::RefCell::new(Vec::new()) };
    static LOG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static IDS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn log(line: String) {
    LOG.with(|l| l.borrow_mut().push(line));
}

/// The window calls so far (this test's thread), emptied.
pub fn take_log() -> Vec<String> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

/// Queue `note` as the window's handlers would.
pub fn queue(note: Note) {
    NOTES.with(|n| n.borrow_mut().push(note));
}

/// Queue `note` as the private window's handlers would.
pub fn queue_private(note: Note) {
    PRIVATE_NOTES.with(|n| n.borrow_mut().push(note));
}

/// Esc in the private window.
pub fn closed_private() {
    VISIBLE.set(false);
    KEY.set(false);
    queue_private(Note::Closed);
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

/// The user clicked another app: the window stays shown without the keyboard.
pub fn blur() {
    KEY.set(false);
}

fn row(r: &Row) -> String {
    match *r {
        Row::Bubble { key, side, header, md, state, .. } => format!("{key} {state:?}|{side:?}|{header}|{md}"),
        Row::Card { key, .. } => format!("{key} card"),
        Row::Divider { text } => format!("-- {text}"),
    }
}

pub const UI: Ui = Ui {
    open: || log("open".into()),
    show: || {
        VISIBLE.set(true);
        KEY.set(true);
        log("show".into());
    },
    hide: || {
        VISIBLE.set(false);
        KEY.set(false);
        log("hide".into());
    },
    visible: || VISIBLE.get(),
    key: || KEY.get(),
    header: |title, subtitle, status| log(format!("header {title}|{subtitle}|{status:?}")),
    rows: |rows| log(format!("rows {}", rows.iter().map(row).collect::<Vec<_>>().join(" / "))),
    notice: |n| {
        if let Some(n) = n {
            log(format!("notice {n}"));
        }
    },
    set_input: |t| log(format!("input {t}")),
    take: || NOTES.with(|n| std::mem::take(&mut *n.borrow_mut())),
    front: || FRONT.get(),
    refocus: |pid| log(format!("refocus {pid}")),
    now: || 1_000,
    new_id: || {
        let n = IDS.get() + 1;
        IDS.set(n);
        format!("lnew{n}")
    },
};

pub const PRIVATE: private_chat::Ui = private_chat::Ui {
    win: Ui { take: || PRIVATE_NOTES.with(|n| std::mem::take(&mut *n.borrow_mut())), ..UI },
    copy: |t| log(format!("copy {t}")),
    hook_quit: |_, _| log("hook quit".into()),
};
