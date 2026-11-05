//! One worker thread per extension: it runs that extension's calls one at a time, in the
//! order queued, leaves each answer in the module's inbox and tells the main thread
//! (`ModuleChanged`). The main thread only queues and drains; it never waits on a child.
//! The thread ends when its `Worker` drops (a reload changed or removed the extension, or
//! the module went away), after the call it is running. An action's card `script` or
//! `flick` runs on a thread of its own (`flick`), its answer in the same inbox.
//!
//! The worker keeps how its last call went (`Last`, for `ext ls`); the main thread copies
//! it under a lock that is held only to copy or store it, never across a call. A raw call
//! (`ext call`) runs in the same queue, its answer sent to the waiting verb, not the inbox.
//!
//! Queries (`op: query`, `op: keyword`) are numbered, newest wins: one waits `DEBOUNCE` after
//! it was queued, on the worker, and runs only if no newer one came; a newer one kills the
//! child that answers an older one (`transport::call`'s `stop`). A superseded query leaves no
//! answer and no `Last`. While a query waits, the calls queued after it (an action, a raw
//! call) run first, so a keystroke never delays them.
//!
//! Events (`op: event`) are fire-and-forget and wait behind every other call: one runs only
//! when nothing else is queued, and a user's call (an action, a query, a raw call) queued
//! while one runs kills its child (`stop`), so a user's call never waits for an event. A
//! killed event leaves no answer and no `Last`, and waits again, to run from the start once
//! nothing else is queued. They coalesce per kind: one waits per kind at most (another of
//! that kind while it waits is dropped), and the worker runs them one at a time. An event
//! that answered with no warnings leaves no answer, only `Last`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::core::later::Answer as Said;

use super::proto::{EventKind, Parsed, Request};
use super::settings::Spec;

mod serve;

/// One call: the extension, the request, `FLICK_SOCKET` and `stop` (true: kill the child).
pub type Run = fn(&Spec, &Request, &Path, &dyn Fn() -> bool) -> Result<Parsed, String>;
/// One run of Flick's own CLI with these words.
pub type Flick = fn(&[String]) -> Result<Parsed, String>;
/// One raw call: the extension, the request text as given and `FLICK_SOCKET`; its stdout.
pub type Raw = fn(&Spec, &str, &Path) -> Result<String, String>;

/// What a worker needs besides its extension.
#[derive(Clone, Copy)]
pub struct Job {
    pub run: Run,
    /// `FLICK_SOCKET`, asked on the worker for each call (it may create the data folder).
    pub socket: fn() -> PathBuf,
    /// Tell the main thread the inbox has an answer.
    pub post: fn(),
    pub flick: Flick,
    pub raw: Raw,
}

/// Run Flick's CLI with `words` on a thread of its own, for action `request` of extension
/// `ext` (a card `script` or `flick`): its answer lands in `inbox` as `request`'s, an error
/// as `<ext>: flick <words>: <why>`.
pub fn flick(job: Job, inbox: Inbox, ext: &str, request: Request, words: Vec<String>) -> Result<(), String> {
    let name = ext.to_string();
    let thread = thread::Builder::new().name(format!("flick-ext-{ext}-run"));
    let run = move || {
        let label = format!("flick {}", words.join(" "));
        let result = (job.flick)(&words).map_err(|e| format!("{name}: {label}: {e}"));
        inbox.push(Answer { ext: name, request, result });
        (job.post)();
    };
    thread.spawn(run).map(drop).map_err(|e| format!("{ext}: cannot start a thread: {e}"))
}

/// A finished call.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub ext: String,
    pub request: Request,
    /// The parsed reply, or the error, which starts with `<ext>: `.
    pub result: Result<Parsed, String>,
}

/// Answers waiting for the main thread, oldest first.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Vec<Answer>>>);

impl Inbox {
    pub fn push(&self, answer: Answer) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(answer);
    }

    pub fn take(&self) -> Vec<Answer> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Answers waiting.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

/// How an extension's last call went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Last {
    /// Its op, `call` for a raw call.
    pub op: &'static str,
    /// Its error, `None` when it answered.
    pub error: Option<String>,
    /// From the spawn to the parsed reply or the error.
    pub took: Duration,
}

/// How long a query waits after it was queued for a newer one (a keystroke).
pub const DEBOUNCE: Duration = Duration::from_millis(60);

/// One entry of a worker's queue.
enum Call {
    Ext(Request),
    /// Query number `.0`, queued at `.1`.
    Query(u64, Instant, Request),
    /// A raw request text and the verb waiting for its reply.
    Raw(String, Sender<Said>),
    /// An event waits (`Shared::waiting`): wake the thread.
    Events,
}

/// The bit of event `kind` in `Shared::waiting`.
fn bit(kind: EventKind) -> u8 {
    1 << EventKind::ALL.iter().position(|k| *k == kind).unwrap_or_default()
}

/// What a worker's thread shares with its `Worker`.
#[derive(Default)]
struct Shared {
    last: Mutex<Option<Last>>,
    /// The number of the newest query; a query with another number is superseded.
    newest: AtomicU64,
    /// The events waiting to run, bit `i` for `EventKind::ALL[i]`.
    waiting: AtomicU8,
    /// User calls queued so far (`send`, `query`, `raw`), counted after each is queued.
    sent: AtomicU64,
}

/// The queue of one extension's worker thread.
pub struct Worker {
    spec: Spec,
    queue: Sender<Call>,
    thread: JoinHandle<()>,
    shared: Arc<Shared>,
}

impl Worker {
    /// Start the thread for `spec`.
    pub fn start(spec: Spec, job: Job, inbox: Inbox) -> Result<Worker, String> {
        let (queue, calls) = mpsc::channel::<Call>();
        let shared = Arc::new(Shared::default());
        let (ext, kept) = (spec.clone(), Arc::clone(&shared));
        let thread = thread::Builder::new().name(format!("flick-ext-{}", spec.name));
        let thread = thread
            .spawn(move || serve::serve(&ext, job, &inbox, &kept, &calls))
            .map_err(|e| format!("{}: cannot start a thread: {e}", spec.name))?;
        Ok(Worker { spec, queue, thread, shared })
    }

    /// Queue user call `call`; `false` when the thread is gone.
    fn put(&self, call: Call) -> bool {
        let queued = self.queue.send(call).is_ok();
        self.shared.sent.fetch_add(1, Ordering::SeqCst);
        queued
    }

    /// How its last finished call went; `None` before the first.
    pub fn last(&self) -> Option<Last> {
        self.shared.last.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    /// Its thread still runs: what it was sent will be answered.
    pub fn alive(&self) -> bool {
        !self.thread.is_finished()
    }

    /// Queue `request`; `false` when the thread is gone.
    pub fn send(&self, request: Request) -> bool {
        self.put(Call::Ext(request))
    }

    /// Queue query `request` (`op: query` or `keyword`), superseding every query sent before
    /// it; `false` when the thread is gone.
    pub fn query(&self, request: Request) -> bool {
        let n = self.shared.newest.fetch_add(1, Ordering::SeqCst) + 1;
        self.put(Call::Query(n, Instant::now(), request))
    }

    /// Supersede every query sent: none of them answers; a running one is killed.
    pub fn cancel(&self) {
        self.shared.newest.fetch_add(1, Ordering::SeqCst);
    }

    /// Queue event `kind`, unless one of that kind waits already; it runs once no other call
    /// waits. `false` when the thread is gone.
    pub fn event(&self, kind: EventKind) -> bool {
        let bit = bit(kind);
        if self.shared.waiting.fetch_or(bit, Ordering::SeqCst) & bit != 0 {
            return self.alive();
        }
        self.queue.send(Call::Events).is_ok()
    }

    /// Queue the raw request `text`; its stdout or error goes to `reply`. `false` when the
    /// thread is gone.
    pub fn raw(&self, text: String, reply: Sender<Said>) -> bool {
        self.put(Call::Raw(text, reply))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ext::proto::Reply;

    fn spec() -> Spec {
        Spec { name: "w".into(), argv: vec![], timeout: Duration::from_secs(1), hotkeys: vec![], keywords: vec![], subscribe: vec![], root: true }
    }

    /// Runs of the "started" event so far.
    static STARTS: AtomicU64 = AtomicU64::new(0);

    fn fake(spec: &Spec, request: &Request, socket: &Path, stop: &dyn Fn() -> bool) -> Result<Parsed, String> {
        match request {
            Request::Root => Ok(Parsed { reply: Reply::Ack, warnings: vec![socket.display().to_string()] }),
            // "slow" runs until it is stopped, bounded.
            Request::Query { query, .. } if query == "slow" => {
                let deadline = Instant::now() + Duration::from_secs(15);
                while !stop() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(format!("{}: stopped {}", spec.name, stop()))
            }
            Request::Query { query, .. } => Ok(Parsed { reply: Reply::Ack, warnings: vec![query.clone()] }),
            Request::Open { view } if view == "slow" => {
                thread::sleep(DEBOUNCE * 3);
                Ok(Parsed { reply: Reply::Ack, warnings: vec![] })
            }
            Request::Event(EventKind::Reloaded) => Ok(Parsed { reply: Reply::Ack, warnings: vec![] }),
            Request::Event(EventKind::Wake) => Ok(Parsed { reply: Reply::Ack, warnings: vec!["woke".into()] }),
            // "started" runs until it is stopped, bounded, the first time; then it answers.
            Request::Event(EventKind::Started) if STARTS.fetch_add(1, Ordering::SeqCst) == 0 => {
                let deadline = Instant::now() + Duration::from_secs(15);
                while !stop() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(format!("{}: stopped {}", spec.name, stop()))
            }
            Request::Event(EventKind::Started) => Ok(Parsed { reply: Reply::Ack, warnings: vec!["again".into()] }),
            _ => Err(format!("{}: {}", spec.name, request.op().name())),
        }
    }

    fn socket() -> PathBuf {
        "/s".into()
    }

    /// Wait (bounded) until `inbox` holds `n` answers, then take them.
    fn wait(inbox: &Inbox, n: usize) -> Vec<Answer> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut got = vec![];
        while got.len() < n && Instant::now() < deadline {
            got.extend(inbox.take());
            thread::sleep(Duration::from_millis(5));
        }
        got
    }

    fn no_flick(words: &[String]) -> Result<Parsed, String> {
        Err(format!("{} words", words.len()))
    }

    /// A raw call answers with its text, or fails for `bad`.
    fn echo(spec: &Spec, text: &str, _: &Path) -> Result<String, String> {
        if text == "bad" { Err(format!("{}: bad", spec.name)) } else { Ok(text.into()) }
    }

    /// Wait (bounded) until `w` has a last call.
    fn last(w: &Worker) -> Option<Last> {
        let deadline = Instant::now() + Duration::from_secs(15);
        while w.last().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        w.last()
    }

    #[test]
    fn a_flick_run_answers_as_its_action() {
        let inbox = Inbox::default();
        let job = Job { run: fake, socket, post: || {}, flick: no_flick, raw: echo };
        let act = Request::Act { view: None, item: "i".into(), action: "a".into() };
        flick(job, inbox.clone(), "w", act.clone(), vec!["task".into(), "ls".into()]).unwrap();
        let got = wait(&inbox, 1);
        assert_eq!((got[0].ext.as_str(), &got[0].request), ("w", &act));
        assert_eq!(got[0].result, Err("w: flick task ls: 2 words".into()));
    }

    #[test]
    fn calls_run_in_order_and_land_in_the_inbox() {
        let inbox = Inbox::default();
        let job = Job { run: fake, socket, post: || {}, flick: no_flick, raw: echo };
        let w = Worker::start(spec(), job, inbox.clone()).unwrap();
        assert_eq!(w.spec().name, "w");
        assert!(w.alive());
        assert!(w.send(Request::Root));
        assert!(w.send(Request::Form { form: "f".into() }));
        let got = wait(&inbox, 2);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].result.as_ref().unwrap().warnings, ["/s"]);
        assert_eq!((got[1].ext.as_str(), got[1].result.clone().unwrap_err()), ("w", "w: form".into()));
        assert_eq!(got[1].request, Request::Form { form: "f".into() });
    }

    #[test]
    fn the_last_call_is_kept_and_a_raw_call_answers_its_caller() {
        let inbox = Inbox::default();
        let job = Job { run: fake, socket, post: || {}, flick: no_flick, raw: echo };
        let w = Worker::start(spec(), job, inbox.clone()).unwrap();
        assert_eq!(w.last(), None);
        assert!(w.send(Request::Form { form: "f".into() }));
        let last = last(&w).unwrap();
        assert_eq!((last.op, last.error.as_deref()), ("form", Some("w: form")));
        let (tx, rx) = mpsc::channel();
        assert!(w.raw("{}".into(), tx.clone()));
        assert_eq!(rx.recv_timeout(Duration::from_secs(15)), Ok(Ok("{}".into())));
        assert!(w.raw("bad".into(), tx));
        assert_eq!(rx.recv_timeout(Duration::from_secs(15)), Ok(Err("w: bad".into())));
        let last = w.last().unwrap();
        assert_eq!((last.op, last.error.as_deref()), ("call", Some("w: bad")));
        assert!(last.took < Duration::from_secs(15));
        // Raw replies never reach the inbox.
        assert_eq!(wait(&inbox, 1).len(), 1);
        assert_eq!(inbox.len(), 0);
    }

    fn query(q: &str) -> Request {
        Request::Query { view: "v".into(), query: q.into() }
    }

    #[test]
    fn only_the_newest_query_answers_and_it_waits_for_the_debounce() {
        let inbox = Inbox::default();
        let job = Job { run: fake, socket, post: || {}, flick: no_flick, raw: echo };
        let w = Worker::start(spec(), job, inbox.clone()).unwrap();
        let started = Instant::now();
        // Queued together: the first two are superseded before they run.
        assert!(w.query(query("a")) && w.query(query("ab")) && w.query(query("abc")));
        let got = wait(&inbox, 1);
        assert!(started.elapsed() >= DEBOUNCE, "{:?}", started.elapsed());
        assert_eq!(got.len(), 1);
        assert_eq!((&got[0].request, &got[0].result.as_ref().unwrap().warnings), (&query("abc"), &vec!["abc".to_string()]));
        assert_eq!(w.last().unwrap().op, "query");
        // A cancelled query never answers; a later call still does.
        assert!(w.query(query("x")));
        w.cancel();
        assert!(w.send(Request::Root));
        let got = wait(&inbox, 1);
        assert_eq!(got[0].request, Request::Root);
        assert_eq!(inbox.len(), 0);
    }

    #[test]
    fn calls_queued_behind_a_waiting_query_run_first() {
        let inbox = Inbox::default();
        let job = Job { run: fake, socket, post: || {}, flick: no_flick, raw: echo };
        let w = Worker::start(spec(), job, inbox.clone()).unwrap();
        // Queued while the slow call runs: the query waits its debounce, the others go first.
        let (tx, rx) = mpsc::channel();
        assert!(w.send(Request::Open { view: "slow".into() }) && w.query(query("q")));
        assert!(w.send(Request::Root) && w.raw("{}".into(), tx));
        let got = wait(&inbox, 3);
        let requests: Vec<&Request> = got.iter().map(|a| &a.request).collect();
        assert_eq!(requests, [&Request::Open { view: "slow".into() }, &Request::Root, &query("q")]);
        assert_eq!(rx.try_recv(), Ok(Ok("{}".into())));
        // The query ran last, after the raw call.
        assert_eq!(w.last().unwrap().op, "query");
    }

    #[test]
    fn a_newer_query_stops_the_running_one() {
        let inbox = Inbox::default();
        let job = Job { run: fake, socket, post: || {}, flick: no_flick, raw: echo };
        let w = Worker::start(spec(), job, inbox.clone()).unwrap();
        assert!(w.query(query("slow")));
        // Most likely running by now; either way only the newer one answers, and fast.
        thread::sleep(DEBOUNCE * 3);
        let started = Instant::now();
        assert!(w.query(query("b")));
        let got = wait(&inbox, 1);
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].request, query("b"));
    }

    #[test]
    fn events_coalesce_per_kind_and_wait_behind_other_calls() {
        let inbox = Inbox::default();
        let job = Job { run: fake, socket, post: || {}, flick: no_flick, raw: echo };
        let w = Worker::start(spec(), job, inbox.clone()).unwrap();
        assert!(w.send(Request::Open { view: "slow".into() }));
        // Queued while the slow call runs: one wake, one unlocked, after the root call.
        assert!(w.event(EventKind::Wake) && w.event(EventKind::Unlocked) && w.event(EventKind::Wake));
        assert!(w.send(Request::Root));
        let got = wait(&inbox, 4);
        let requests: Vec<&Request> = got.iter().map(|a| &a.request).collect();
        let want = [Request::Open { view: "slow".into() }, Request::Root, Request::Event(EventKind::Wake), Request::Event(EventKind::Unlocked)];
        assert_eq!(requests, want.iter().collect::<Vec<_>>());
        assert_eq!(got[3].result, Err("w: event".into()));
        // A quiet answer leaves only `Last`.
        assert!(w.event(EventKind::Reloaded));
        let deadline = Instant::now() + Duration::from_secs(15);
        while w.last().is_some_and(|l| l.error.is_some()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!((w.last().unwrap().op, w.last().unwrap().error), ("event", None));
        assert_eq!(inbox.len(), 0);
    }

    #[test]
    fn a_users_call_stops_the_running_event_which_runs_again_after_it() {
        let inbox = Inbox::default();
        let job = Job { run: fake, socket, post: || {}, flick: no_flick, raw: echo };
        let w = Worker::start(spec(), job, inbox.clone()).unwrap();
        assert!(w.event(EventKind::Started));
        let deadline = Instant::now() + Duration::from_secs(15);
        while STARTS.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        // Queued while the event runs: it is killed, the call answers, then the event again.
        let started = Instant::now();
        assert!(w.send(Request::Root));
        let got = wait(&inbox, 2);
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        let requests: Vec<&Request> = got.iter().map(|a| &a.request).collect();
        assert_eq!(requests, [&Request::Root, &Request::Event(EventKind::Started)]);
        assert_eq!(got[1].result.as_ref().unwrap().warnings, ["again"]);
        assert_eq!(STARTS.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_worker_whose_thread_died_refuses_calls() {
        fn panics(_: &Spec, _: &Request, _: &Path, _: &dyn Fn() -> bool) -> Result<Parsed, String> {
            panic!("extension call panicked")
        }
        let w = Worker::start(spec(), Job { run: panics, socket, post: || {}, flick: no_flick, raw: echo }, Inbox::default());
        let w = w.unwrap();
        assert!(w.send(Request::Root));
        let deadline = Instant::now() + Duration::from_secs(15);
        while (w.send(Request::Root) || w.alive()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!w.send(Request::Root));
        assert!(!w.alive());
        assert!(!w.event(EventKind::Wake) && !w.event(EventKind::Wake));
    }
}
