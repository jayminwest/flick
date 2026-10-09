//! One screenshot at a time on a worker thread (the rebuild module's background pattern).
//! Interactive captures wait for the user, so they never run on the main thread: the worker
//! runs the shutter, leaves the result in shared state and posts `ModuleChanged`; the module
//! drains it on the main thread.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::platform::capture::{Error, Request, Shot};

/// A screenshot to take, and what to do with it once taken.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub req: Request,
    /// The store's `kind`: `area`, `window`, `screen`, `display` or `rect`.
    pub kind: &'static str,
    /// Put the image on the clipboard.
    pub copy: bool,
    /// Record a row in `capture_shots` (false for a clipboard-only temp file).
    pub record: bool,
    /// Open the annotation editor on the shot once it lands.
    pub annotate: bool,
    /// Pause before the shutter, so a launcher hidden just now is off the screen.
    pub wait: Duration,
}

/// A finished job.
#[derive(Debug)]
pub struct Done {
    pub job: Job,
    pub result: Result<Shot, Error>,
}

#[derive(Default)]
struct State {
    running: bool,
    done: Vec<Done>,
}

/// The worker's shared state: whether a capture runs, and results not drained yet.
#[derive(Default)]
pub struct Worker(Arc<Mutex<State>>);

impl Worker {
    /// A capture is running.
    pub fn busy(&self) -> bool {
        self.0.lock().is_ok_and(|s| s.running)
    }

    /// Run `job` with `shoot` on a thread, then call `notify`. Refuses while one runs.
    pub fn start(
        &self,
        job: Job,
        shoot: fn(&Request) -> Result<Shot, Error>,
        notify: fn(),
    ) -> Result<(), String> {
        {
            let mut state = self.0.lock().map_err(|_| "capture: worker state is poisoned")?;
            if state.running {
                return Err("A capture is already running".into());
            }
            state.running = true;
        }
        let shared = self.0.clone();
        let spawned = std::thread::Builder::new().name("flick-capture".into()).spawn(move || {
            std::thread::sleep(job.wait);
            let result = shoot(&job.req);
            let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
            state.done.push(Done { job, result });
            state.running = false;
            drop(state);
            notify();
        });
        spawned.map(drop).map_err(|e| {
            if let Ok(mut state) = self.0.lock() {
                state.running = false;
            }
            format!("capture: cannot start: {e}")
        })
    }

    /// Finished jobs, oldest first; they are removed.
    pub fn drain(&self) -> Vec<Done> {
        self.0.lock().map(|mut s| std::mem::take(&mut s.done)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::capture::Target;
    use std::time::Instant;

    fn job() -> Job {
        let req = Request {
            target: Target::Area,
            path: "/nonexistent/flick/a.png".into(),
            cursor: false,
            shadow: true,
            sound: false,
        };
        Job { req, kind: "area", copy: true, record: true, annotate: false, wait: Duration::ZERO }
    }

    fn settle(w: &Worker) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while w.busy() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_job_runs_once_and_drains() {
        let w = Worker::default();
        assert!(!w.busy() && w.drain().is_empty());
        w.start(job(), |_| Err(Error::Cancelled), || {}).unwrap();
        settle(&w);
        let done = w.drain();
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].job.kind, &done[0].result), ("area", &Err(Error::Cancelled)));
        assert!(w.drain().is_empty());
    }

    #[test]
    fn a_second_job_waits_for_the_first() {
        let w = Worker::default();
        let slow = |_: &Request| {
            std::thread::sleep(Duration::from_millis(200));
            Err(Error::Failed("slow".into()))
        };
        w.start(job(), slow, || {}).unwrap();
        assert_eq!(w.start(job(), slow, || {}).unwrap_err(), "A capture is already running");
        settle(&w);
        assert_eq!(w.drain().len(), 1);
    }
}
