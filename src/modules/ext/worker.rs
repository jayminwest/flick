//! One worker thread per extension: it runs that extension's calls one at a time, in the
//! order queued, leaves each answer in the module's inbox and tells the main thread
//! (`ModuleChanged`). The main thread only queues and drains; it never waits on a child.
//! The thread ends when its `Worker` drops (a reload changed or removed the extension, or
//! the module went away), after the call it is running.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use super::proto::{Parsed, Request};
use super::settings::Spec;

/// One call: the extension, the request and `FLICK_SOCKET`.
pub type Run = fn(&Spec, &Request, &Path) -> Result<Parsed, String>;

/// What a worker needs besides its extension.
#[derive(Clone, Copy)]
pub struct Job {
    pub run: Run,
    /// `FLICK_SOCKET`, asked on the worker for each call (it may create the data folder).
    pub socket: fn() -> PathBuf,
    /// Tell the main thread the inbox has an answer.
    pub post: fn(),
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
    fn push(&self, answer: Answer) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(answer);
    }

    pub fn take(&self) -> Vec<Answer> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The queue of one extension's worker thread.
pub struct Worker {
    spec: Spec,
    queue: Sender<Request>,
}

impl Worker {
    /// Start the thread for `spec`.
    pub fn start(spec: Spec, job: Job, inbox: Inbox) -> Result<Worker, String> {
        let (queue, calls) = mpsc::channel::<Request>();
        let ext = spec.clone();
        let thread = thread::Builder::new().name(format!("flick-ext-{}", spec.name));
        thread
            .spawn(move || {
                for request in calls {
                    let result = (job.run)(&ext, &request, &(job.socket)());
                    inbox.push(Answer { ext: ext.name.clone(), request, result });
                    (job.post)();
                }
            })
            .map_err(|e| format!("{}: cannot start a thread: {e}", spec.name))?;
        Ok(Worker { spec, queue })
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    /// Queue `request`; `false` when the thread is gone.
    pub fn send(&self, request: Request) -> bool {
        self.queue.send(request).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ext::proto::Reply;
    use std::time::{Duration, Instant};

    fn spec() -> Spec {
        Spec { name: "w".into(), argv: vec![], timeout: Duration::from_secs(1) }
    }

    fn fake(spec: &Spec, request: &Request, socket: &Path) -> Result<Parsed, String> {
        match request {
            Request::Root => Ok(Parsed { reply: Reply::Ack, warnings: vec![socket.display().to_string()] }),
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

    #[test]
    fn calls_run_in_order_and_land_in_the_inbox() {
        let inbox = Inbox::default();
        let job = Job { run: fake, socket, post: || {} };
        let w = Worker::start(spec(), job, inbox.clone()).unwrap();
        assert_eq!(w.spec().name, "w");
        assert!(w.send(Request::Root));
        assert!(w.send(Request::Form { form: "f".into() }));
        let got = wait(&inbox, 2);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].result.as_ref().unwrap().warnings, ["/s"]);
        assert_eq!((got[1].ext.as_str(), got[1].result.clone().unwrap_err()), ("w", "w: form".into()));
        assert_eq!(got[1].request, Request::Form { form: "f".into() });
    }

    #[test]
    fn a_worker_whose_thread_died_refuses_calls() {
        fn panics(_: &Spec, _: &Request, _: &Path) -> Result<Parsed, String> {
            panic!("extension call panicked")
        }
        let w = Worker::start(spec(), Job { run: panics, socket, post: || {} }, Inbox::default());
        let w = w.unwrap();
        assert!(w.send(Request::Root));
        let deadline = Instant::now() + Duration::from_secs(15);
        while w.send(Request::Root) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!w.send(Request::Root));
    }
}
