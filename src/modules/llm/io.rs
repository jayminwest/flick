//! The module's background work: every curl runs on a named thread started here
//! (`transport::run`); the main thread only starts threads and reads `State` under a short
//! lock (or, for `llm ping|models`, waits on it for at most `ASK_WAIT`).
//!
//! - `llm-models`: one `GET /v1/models` of one server; the list (or the error) and how long
//!   it took land in `State::models` under the server's name. One at a time per server.
//! - `llm-stream`: one streamed chat reply; each line's pieces land in its `Stream`'s inbox.
//! - `llm-watchdog`: kills a call still running after its budget (curl's own `--max-time`
//!   should end it first).
//!
//! Each write notifies waiters and posts `ModuleChanged`, at most one undelivered at a time
//! (`State::posted`, cleared by the module when the event arrives), so a fast stream does
//! not flood the main queue. Nothing here logs: replies and prompts never reach a log line.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::openai::{self, Line, Model, Piece};
use super::settings::Server;
use super::transport::{self, Call, ChildRef, Ended, Spawn};

/// curl's `--max-time` for a model list.
pub const LIST_MAX_TIME: u64 = 3;
/// The watchdog's margin past curl's `--max-time`.
const WATCHDOG_MARGIN: u64 = 2;
/// The most of a plain (non-stream) reply body kept to explain an error.
const RAW_CAP: usize = 4_096;

/// What the threads need from outside; tests swap in fakes (`testkit`).
#[derive(Clone, Copy)]
pub struct Hooks {
    /// Start curl (`wire::spawn`).
    pub spawn: Spawn,
    /// Post `ModuleChanged` for this module (any thread).
    pub post: fn(),
    /// Sleep on the watchdog thread.
    pub sleep: fn(Duration),
}

/// One server's model list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Models {
    /// Bumped each time a fetch ends.
    pub seq: u64,
    pub fetching: bool,
    /// The last fetch's list, or why it failed; `None` before the first.
    pub result: Option<Result<Vec<Model>, String>>,
    /// How long the last fetch took.
    pub took: Duration,
}

/// Where a chat reply is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

/// One chat reply in flight or ended but not yet taken.
struct Stream {
    id: u64,
    pieces: Vec<Piece>,
    status: Status,
    child: Option<ChildRef>,
}

/// What the threads write and the main thread reads.
#[derive(Default)]
pub struct State {
    pub models: HashMap<String, Models>,
    streams: Vec<Stream>,
    next_id: u64,
    /// A `ModuleChanged` is posted and not yet delivered.
    pub posted: bool,
    /// Bumped by `stop`: threads from before write nothing.
    epoch: u64,
}

impl State {
    fn stream(&mut self, id: u64) -> Option<&mut Stream> {
        self.streams.iter_mut().find(|s| s.id == id)
    }
}

#[derive(Default)]
pub struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    /// A poisoned lock still holds usable data.
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wake waiters and post `ModuleChanged` unless one is on its way.
    fn changed(&self, mut st: MutexGuard<'_, State>, hooks: Hooks) {
        let post = !std::mem::replace(&mut st.posted, true);
        drop(st);
        self.changed.notify_all();
        if post {
            (hooks.post)();
        }
    }

    /// Wait until `done` holds or `budget` runs out; whether it holds.
    pub fn wait(&self, budget: Duration, done: impl Fn(&State) -> bool) -> bool {
        let deadline = Instant::now() + budget;
        let mut st = self.lock();
        while !done(&st) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            st = self.changed.wait_timeout(st, left).unwrap_or_else(PoisonError::into_inner).0;
        }
        true
    }

    /// Kill every call and drop every stream (module dropped or disabled).
    pub fn stop(&self) {
        let mut st = self.lock();
        st.epoch += 1;
        for s in std::mem::take(&mut st.streams) {
            if let Some(child) = s.child {
                transport::lock(&child).kill();
            }
        }
        for m in st.models.values_mut() {
            m.fetching = false;
        }
    }

    /// Take a stream's new pieces and its status; an ended stream is forgotten once taken.
    /// `None`: no such stream. The caller owns the text now (and wipes it if private).
    #[cfg_attr(not(test), expect(dead_code, reason = "the chat window reads replies (flick-6a0d)"))]
    pub fn take(&self, id: u64) -> Option<(Vec<Piece>, Status)> {
        let mut st = self.lock();
        let i = st.streams.iter().position(|s| s.id == id)?;
        let s = &mut st.streams[i];
        let out = (std::mem::take(&mut s.pieces), s.status.clone());
        if out.1 != Status::Running {
            st.streams.remove(i);
        }
        Some(out)
    }
}

/// Kill `child` after `secs` unless `ended` says the call is over; `on_kill` runs under
/// the lock first (it marks why).
fn watchdog(
    shared: &Arc<Shared>,
    child: &ChildRef,
    secs: u64,
    hooks: Hooks,
    on_kill: impl FnOnce(&mut State) -> bool + Send + 'static,
) {
    let (sh, child) = (Arc::clone(shared), Arc::clone(child));
    let _ = thread::Builder::new().name("llm-watchdog".into()).spawn(move || {
        (hooks.sleep)(Duration::from_secs(secs));
        let mut st = sh.lock();
        if on_kill(&mut st) {
            drop(st);
            transport::lock(&child).kill();
        }
    });
}

/// Fetch `server`'s model list on a thread unless a fetch runs. Returns the `Models::seq`
/// that the fetch in flight will reach.
pub fn fetch_models(shared: &Arc<Shared>, server: &Server, hooks: Hooks) -> u64 {
    let name = server.name.clone();
    let epoch = {
        let mut st = shared.lock();
        let epoch = st.epoch;
        let m = st.models.entry(name.clone()).or_default();
        if m.fetching {
            return m.seq + 1;
        }
        m.fetching = true;
        epoch
    };
    let url = server.endpoint("models");
    let sh = Arc::clone(shared);
    let started = Instant::now();
    let spawned = thread::Builder::new().name("llm-models".into()).spawn(move || {
        let mut body = String::new();
        let call = Call { url, body: None, max_time: LIST_MAX_TIME };
        let budget = LIST_MAX_TIME + WATCHDOG_MARGIN;
        // Killing a curl that already ended does nothing: it is reaped, so no other process
        // can have its pid.
        let ended = transport::run(call, hooks.spawn, |c| watchdog(&sh, c, budget, hooks, |_| true), |l| {
            if body.len() < 1 << 20 {
                body.push_str(l);
                body.push('\n');
            }
        });
        let result = match ended {
            Err(e) => Err(e),
            Ok(end) => match (end.failure(), openai::models(&body)) {
                (None, list) => list,
                (Some(fail), list) => Err(list.err().filter(|_| end.code == Some(22)).unwrap_or(fail)),
            },
        };
        let mut st = sh.lock();
        if st.epoch != epoch {
            return;
        }
        let m = st.models.entry(name).or_default();
        *m = Models { seq: m.seq + 1, fetching: false, result: Some(result), took: started.elapsed() };
        sh.changed(st, hooks);
    });
    let mut st = shared.lock();
    let m = st.models.entry(server.name.clone()).or_default();
    if spawned.is_err() {
        m.fetching = false;
        m.seq += 1;
        m.result = Some(Err("no thread for curl".into()));
        return m.seq;
    }
    m.seq + 1
}

/// What a stream's reader saw, for its verdict.
#[derive(Default)]
struct Seen {
    done: bool,
    data: bool,
    error: Option<String>,
    raw: String,
}

impl Seen {
    fn status(&self, ended: Result<Ended, String>) -> Status {
        if let Some(e) = &self.error {
            return Status::Failed(e.clone());
        }
        let ended = match ended {
            Ok(end) => end,
            Err(e) => return Status::Failed(e),
        };
        match ended.failure() {
            None if self.done || self.data => Status::Done,
            None => Status::Failed(openai::error_body(&self.raw).unwrap_or_else(|| "the server sent no reply".into())),
            Some(fail) => Status::Failed(openai::error_body(&self.raw).unwrap_or(fail)),
        }
    }
}

/// Stream a chat reply: POST `body` (from `openai::chat_body`, wiped once sent) to
/// `server`'s `/v1/chat/completions`, within `timeout` seconds. Returns the stream id for
/// `Shared::take` and `cancel`.
#[cfg_attr(not(test), expect(dead_code, reason = "the chat window sends prompts (flick-6a0d)"))]
pub fn chat(shared: &Arc<Shared>, server: &Server, body: Vec<u8>, timeout: u64, hooks: Hooks) -> u64 {
    let id = {
        let mut st = shared.lock();
        st.next_id += 1;
        let id = st.next_id;
        st.streams.push(Stream { id, pieces: vec![], status: Status::Running, child: None });
        id
    };
    let call = Call { url: server.endpoint("chat/completions"), body: Some(body), max_time: timeout };
    let sh = Arc::clone(shared);
    let spawned = thread::Builder::new().name("llm-stream".into()).spawn(move || {
        let mut seen = Seen::default();
        let started = |c: &ChildRef| {
            let mut st = sh.lock();
            if let Some(s) = st.stream(id) {
                s.child = Some(Arc::clone(c));
            }
            drop(st);
            let late = move |st: &mut State| match st.stream(id).filter(|s| s.status == Status::Running) {
                Some(s) => {
                    s.status = Status::Failed(format!("timed out after {timeout} s"));
                    true
                }
                None => false,
            };
            watchdog(&sh, c, timeout + WATCHDOG_MARGIN, hooks, late);
        };
        let ended = transport::run(call, hooks.spawn, started, |l| match openai::sse_line(l) {
            Line::Skip => {}
            Line::Other => {
                if seen.raw.len() < RAW_CAP {
                    seen.raw.push_str(l);
                }
            }
            Line::Data(pieces) => {
                seen.data = true;
                seen.done |= pieces.contains(&Piece::Done);
                if let Some(Piece::Error(e)) = pieces.iter().find(|p| matches!(p, Piece::Error(_))) {
                    seen.error.get_or_insert_with(|| e.clone());
                }
                // After `stop` the stream is gone, so a late line lands nowhere.
                let mut st = sh.lock();
                if let Some(s) = st.stream(id).filter(|s| s.status == Status::Running) {
                    s.pieces.extend(pieces);
                    sh.changed(st, hooks);
                }
            }
        });
        let status = seen.status(ended);
        transport::wipe_string(&mut seen.raw);
        let mut st = sh.lock();
        if let Some(s) = st.stream(id) {
            s.child = None;
            if s.status == Status::Running {
                s.status = status;
            }
            sh.changed(st, hooks);
        }
    });
    if spawned.is_err() {
        let mut st = shared.lock();
        if let Some(s) = st.stream(id) {
            s.status = Status::Failed("no thread for curl".into());
        }
    }
    id
}

/// Stop stream `id`: kill its curl, keep what arrived. False when it is not running.
#[cfg_attr(not(test), expect(dead_code, reason = "the chat window's stop (flick-6a0d)"))]
pub fn cancel(shared: &Shared, id: u64) -> bool {
    let mut st = shared.lock();
    let Some(s) = st.stream(id).filter(|s| s.status == Status::Running) else { return false };
    s.status = Status::Cancelled;
    let child = s.child.clone();
    drop(st);
    if let Some(child) = child {
        transport::lock(&child).kill();
    }
    shared.changed.notify_all();
    true
}

#[cfg(test)]
mod tests;
