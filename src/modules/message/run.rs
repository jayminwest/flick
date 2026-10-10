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

/// Results not drained yet, shared with the worker threads.
#[derive(Clone, Default)]
pub struct Worker(Arc<Mutex<Vec<Done>>>);

impl Worker {
    /// Run `job` with `exec` on a thread, then call `notify`. A thread that cannot start is
    /// a failed press, delivered at once.
    pub fn start(&self, job: Job, exec: fn(&Job) -> Exit, notify: fn()) {
        let shared = self.clone();
        let (card, press) = (job.card.clone(), job.press);
        let spawned = thread::Builder::new().name("flick-kota-action".into()).spawn(move || {
            let exit = exec(&job);
            shared.push(Done { card: job.card, press: job.press, exit });
            notify();
        });
        if let Err(e) = spawned {
            self.push(Done { card, press, exit: Exit::Failed(format!("cannot start the action: {e}")) });
        }
    }

    fn push(&self, done: Done) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(done);
    }

    /// Finished presses not taken yet.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// Every finished press, oldest first.
    pub fn take(&self) -> Vec<Done> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Run `job.argv` with `job.stdin`, killed after `job.budget`.
pub fn exec(job: &Job) -> Exit {
    let Some((program, args)) = job.argv.split_first() else {
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
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(job.stdin.as_bytes());
    }
    let (tx, rx) = mpsc::channel();
    if let Some(mut stderr) = child.stderr.take() {
        thread::spawn(move || {
            let mut out = String::new();
            let _ = stderr.read_to_string(&mut out);
            let _ = tx.send(out);
        });
    }
    let deadline = Instant::now() + job.budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Exit::Failed(format!("No answer from KOTA in {} s", job.budget.as_secs()));
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
fn last_line(text: &str) -> Option<String> {
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
    fn the_worker_runs_on_a_thread_and_queues_results() {
        let w = Worker::default();
        w.start(Job::new(&["x".into()], "c1", "go", 3, String::new()), |_| Exit::Sent, || {});
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut done = w.take();
        while done.is_empty() && Instant::now() < deadline {
            thread::sleep(POLL);
            done = w.take();
        }
        assert_eq!(done, [Done { card: "c1".into(), press: 3, exit: Exit::Sent }]);
        assert!(w.take().is_empty());
    }
}
