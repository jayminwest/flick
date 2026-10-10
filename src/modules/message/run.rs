//! The KOTA reply worker: a press on a card's reply action runs `[message] action_command`
//! with `--action --card <id> --action-id <aid>` appended and the values JSON on stdin, on a
//! named thread with a hard budget (the rebuild module's background pattern). The result
//! lands in the worker's shared list and the worker posts `ModuleChanged`; the module drains
//! it on the main thread.
//!
//! Exit 0: sent (the card waits for KOTA to re-post it). Exit 2: KOTA rejected the press,
//! its last stderr line says why. Anything else (another exit, a signal, the budget, a spawn
//! failure) is a failure the user may retry.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Time one press may take, from spawn to exit; past it the command is killed.
pub const BUDGET: Duration = Duration::from_secs(20);
/// How often the worker checks its child.
const POLL: Duration = Duration::from_millis(10);
/// The error line when a worker thread cannot start.
pub const NO_THREAD: &str = "Cannot start the action: no thread";
/// Longest stderr line kept for an error, in chars.
const LINE_MAX: usize = 200;

/// One press to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub card: String,
    pub action: String,
    /// Which press of the card this is; a result for an older press is ignored.
    pub press: u64,
    /// `action_command` plus `--action --card <id> --action-id <aid>`.
    pub argv: Vec<String>,
    /// The values JSON, written to stdin.
    pub stdin: String,
    pub budget: Duration,
}

impl Job {
    pub fn new(command: &[String], card: &str, action: &str, press: u64, values: String) -> Job {
        let extra = ["--action", "--card", card, "--action-id", action].map(String::from);
        Job {
            card: card.into(),
            action: action.into(),
            press,
            argv: command.iter().cloned().chain(extra).collect(),
            stdin: values,
            budget: BUDGET,
        }
    }
}

/// How a press ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    /// Exit 0: KOTA has it; the card waits for KOTA's update.
    Sent,
    /// Exit 2: KOTA refused it, with the last stderr line.
    Rejected(String),
    /// Anything else, as the error line the card shows.
    Failed(String),
}

/// A finished press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    pub card: String,
    pub press: u64,
    pub exit: Exit,
}

/// Results not drained yet, shared with the worker threads: KOTA sends (`Done`) or local
/// runs (`local::Done`).
pub struct Worker<D>(Arc<Mutex<Vec<D>>>);

impl<D> Default for Worker<D> {
    fn default() -> Self {
        Worker(Arc::default())
    }
}

impl<D> Clone for Worker<D> {
    fn clone(&self) -> Self {
        Worker(Arc::clone(&self.0))
    }
}

impl<D: Send + 'static> Worker<D> {
    /// Run `work` on a thread named `name`, queue its result, then call `notify`. A thread
    /// that cannot start queues `failed` at once.
    pub fn spawn(&self, name: &str, work: impl FnOnce() -> D + Send + 'static, failed: D, notify: fn()) {
        let shared = self.clone();
        let spawned = thread::Builder::new().name(name.into()).spawn(move || {
            shared.push(work());
            notify();
        });
        if spawned.is_err() {
            self.push(failed);
        }
    }

    fn push(&self, done: D) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(done);
    }

    /// Finished results not taken yet.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// Whether a worker thread still holds the queue (its result may be on the way).
    #[cfg(test)]
    pub fn running(&self) -> bool {
        Arc::strong_count(&self.0) > 1
    }

    /// Every finished result, oldest first.
    pub fn take(&self) -> Vec<D> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Worker<Done> {
    /// Send `job` with `exec` on a thread, then call `notify`.
    pub fn start(&self, job: Job, exec: fn(&Job) -> Exit, notify: fn()) {
        let failed = Done { card: job.card.clone(), press: job.press, exit: Exit::Failed(NO_THREAD.into()) };
        let work = move || {
            let exit = exec(&job);
            Done { card: job.card, press: job.press, exit }
        };
        self.spawn("flick-kota-action", work, failed, notify);
    }
}

/// Run `job.argv` with `job.stdin`, killed after `job.budget`.
pub fn exec(job: &Job) -> Exit {
    exec_bytes(&job.argv, job.stdin.as_bytes().to_vec(), job.budget)
}

/// Run `command` (an argv) with `input` on stdin, killed after `budget`. It is written on its own thread, so
/// a child that stops reading (a stalled ssh) can't block past the budget: the kill closes the
/// pipe and the write fails.
pub fn exec_bytes(command: &[String], input: Vec<u8>, budget: Duration) -> Exit {
    let Some((program, args)) = command.split_first() else {
        return Exit::Failed("action_command is empty".into());
    };
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return Exit::Failed(format!("action_command: {program}: {e}")),
    };
    // A child that exits without reading stdin closes the pipe: the write error is moot.
    let pipe = child.stdin.take();
    thread::spawn(move || pipe.map(|mut p| p.write_all(&input)));
    let (tx, rx) = mpsc::channel();
    if let Some(mut stderr) = child.stderr.take() {
        thread::spawn(move || {
            let mut out = String::new();
            let _ = stderr.read_to_string(&mut out);
            let _ = tx.send(out);
        });
    }
    let deadline = Instant::now() + budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Exit::Failed(format!("No answer from KOTA in {} s", budget.as_secs()));
            }
            Err(e) => return Exit::Failed(format!("action_command: {e}")),
        }
    };
    // A grandchild (ssh's) may keep stderr open; never wait on it for long.
    let stderr = rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
    classify(status.code(), &stderr)
}

/// The outcome of an exit `code` (`None`: killed by a signal) with `stderr`.
pub fn classify(code: Option<i32>, stderr: &str) -> Exit {
    let line = last_line(stderr);
    match code {
        Some(0) => Exit::Sent,
        Some(2) => Exit::Rejected(line.unwrap_or_else(|| "no reason given".into())),
        Some(n) => Exit::Failed(match line {
            Some(l) => format!("Sending to KOTA failed (exit {n}): {l}"),
            None => format!("Sending to KOTA failed (exit {n})"),
        }),
        None => Exit::Failed("Sending to KOTA failed: killed by a signal".into()),
    }
}

/// The last non-empty line of `text`, trimmed and cut to `LINE_MAX` chars.
pub fn last_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    let mut out: String = line.chars().take(LINE_MAX).collect();
    if line.chars().count() > LINE_MAX {
        out.push('…');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str, budget: Duration) -> Job {
        let command = ["/bin/sh", "-c", script, "sh"].map(String::from);
        Job { budget, ..Job::new(&command, "c1", "go", 1, r#"{"x":"y"}"#.into()) }
    }

    #[test]
    fn a_job_appends_the_card_and_action() {
        let job = Job::new(&["kota-ask".into()], "c1", "go", 7, "{}".into());
        assert_eq!(job.argv, ["kota-ask", "--action", "--card", "c1", "--action-id", "go"]);
        assert_eq!((job.press, job.budget, job.stdin.as_str()), (7, BUDGET, "{}"));
    }

    #[test]
    fn exit_codes_map_to_outcomes() {
        assert_eq!(classify(Some(0), "noise"), Exit::Sent);
        assert_eq!(classify(Some(2), "a\n  card is gone  \n\n"), Exit::Rejected("card is gone".into()));
        assert_eq!(classify(Some(2), ""), Exit::Rejected("no reason given".into()));
        assert_eq!(classify(Some(1), "boom"), Exit::Failed("Sending to KOTA failed (exit 1): boom".into()));
        assert_eq!(classify(Some(255), " "), Exit::Failed("Sending to KOTA failed (exit 255)".into()));
        assert_eq!(classify(None, ""), Exit::Failed("Sending to KOTA failed: killed by a signal".into()));
        let long = last_line(&"x".repeat(300)).unwrap();
        assert_eq!(long.chars().count(), LINE_MAX + 1);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn exec_passes_args_and_stdin_and_reads_the_exit() {
        let budget = Duration::from_secs(10);
        // The values arrive on stdin and the appended args after the command's own.
        let echo = sh(r#"read -r v; [ "$v" = '{"x":"y"}' ] && [ "$*" = '--action --card c1 --action-id go' ] && exit 0; exit 1"#, budget);
        assert_eq!(exec(&echo), Exit::Sent);
        assert_eq!(exec(&sh("echo first >&2; echo 'not now' >&2; exit 2", budget)), Exit::Rejected("not now".into()));
        assert_eq!(exec(&sh("exit 3", budget)), Exit::Failed("Sending to KOTA failed (exit 3)".into()));
        assert_eq!(exec(&sh("kill -9 $$", budget)), Exit::Failed("Sending to KOTA failed: killed by a signal".into()));
        // Not reading stdin is fine.
        assert_eq!(exec(&sh("exit 0", budget)), Exit::Sent);
    }

    #[test]
    fn exec_kills_at_the_budget_and_reports_spawn_failures() {
        let started = Instant::now();
        let slow = Job { budget: Duration::from_millis(200), ..sh("sleep 5", Duration::ZERO) };
        assert_eq!(exec(&slow), Exit::Failed("No answer from KOTA in 0 s".into()));
        assert!(started.elapsed() < Duration::from_secs(4));
        let missing = Job::new(&["/nonexistent/kota-ask".into()], "c1", "go", 1, String::new());
        let Exit::Failed(e) = exec(&missing) else { panic!("spawned") };
        assert!(e.starts_with("action_command: /nonexistent/kota-ask: "), "{e}");
        let empty = Job { argv: vec![], ..missing };
        assert_eq!(exec(&empty), Exit::Failed("action_command is empty".into()));
    }

    #[test]
    fn exec_bytes_sends_binary_stdin_and_never_blocks_on_a_child_that_stops_reading() {
        let argv = |script: &str| ["/bin/sh", "-c", script].map(String::from).to_vec();
        // A megabyte with NULs and high bytes arrives whole.
        let png: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
        let budget = Duration::from_secs(10);
        assert_eq!(exec_bytes(&argv("[ \"$(wc -c | tr -d ' ')\" = 1048576 ]"), png.clone(), budget), Exit::Sent);
        // Far more than a pipe holds, to a child that never reads: the budget still holds.
        let started = Instant::now();
        let stuck = exec_bytes(&argv("sleep 5"), png, Duration::from_millis(200));
        assert_eq!(stuck, Exit::Failed("No answer from KOTA in 0 s".into()));
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn the_worker_runs_on_a_thread_and_queues_results() {
        // No polling: the job waits at a gate the test holds, and `notify` signals a condvar,
        // so every line runs the same way however the threads are scheduled (flick-2b94).
        static GATE: Mutex<()> = Mutex::new(());
        static NOTIFIED: (Mutex<bool>, std::sync::Condvar) = (Mutex::new(false), std::sync::Condvar::new());
        let w = Worker::default();
        let gate = GATE.lock().unwrap();
        let exec = |_: &Job| {
            drop(GATE.lock().unwrap_or_else(PoisonError::into_inner));
            Exit::Sent
        };
        let notify = || {
            *NOTIFIED.0.lock().unwrap() = true;
            NOTIFIED.1.notify_all();
        };
        w.start(Job::new(&["x".into()], "c1", "go", 3, String::new()), exec, notify);
        assert!(w.take().is_empty(), "the job is held at the gate");
        drop(gate);
        let wait = NOTIFIED.1.wait_timeout_while(NOTIFIED.0.lock().unwrap(), Duration::from_secs(15), |n| !*n);
        assert!(*wait.unwrap().0, "notified");
        assert_eq!(w.take(), [Done { card: "c1".into(), press: 3, exit: Exit::Sent }]);
        assert!(w.take().is_empty());
    }
}
