//! Local actions that run a process (plan flick-7da1, step 10): `script` and `flick` re-run
//! the Flick binary as a command line client, `shell` runs `/bin/sh -c`. Each runs on a
//! named worker thread (`run::Worker`) with a hard budget; past it the process group is
//! killed. The card shows a short result line, never the output, and nothing is stored.
//!
//! Self-exec, not `app::control`: `<exe> script run <name> [query]` and `<exe> <words...>`
//! are ordinary requests over the control socket, so they take the normal control path
//! (`Registry::command`, its panic guard, every module's own checks) and the module never
//! reaches into another. `std::env::current_exe()` is `Flick.app/Contents/MacOS/Flick` when
//! Flick runs as an app; with arguments that binary is the CLI, which connects to the
//! running Flick's socket. That request runs on the main thread (`control::run` posts it
//! with `events::on_main`); the socket thread waits for it, and the worker thread here
//! waits for the child. The main thread never waits on either, so it is free to answer and
//! there is no deadlock.
//!
//! The child gets no stdin, `$FLICK_HOST` removed (it must talk to this Mac's Flick), and
//! `$FLICK_REMOTE` set only for a card posted over the network, so module-level remote
//! guards (`Cx::remote`) apply to it too, on top of `Do::check`'s `net_policy`.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::run::last_line;
use crate::core::card::{Do, Origin};

/// Time a `shell` command may take before it is killed.
pub const SHELL_BUDGET: Duration = Duration::from_secs(60);
/// Time a `script` or `flick` request may take. `script run` only starts the script, and
/// a request answers in milliseconds, so this is generous.
pub const FLICK_BUDGET: Duration = Duration::from_secs(30);
/// How often the worker checks its child.
const POLL: Duration = Duration::from_millis(10);
/// Bytes of each output stream kept, from the end: enough for the last line.
const TAIL: usize = 4096;
/// Longest label in "Running …", in chars.
const LABEL_MAX: usize = 60;

/// One local run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub card: String,
    /// The card's press number; a result for an older press is ignored.
    pub press: u64,
    pub argv: Vec<String>,
    pub budget: Duration,
    /// What runs, for the card's lines: `script deploy`, `flick task ls`, `command`.
    pub label: String,
    /// Run in the home folder (a shell command) rather than Flick's working directory.
    pub home: bool,
    /// `None`: a shell command, environment untouched. `Some`: a self-exec, `$FLICK_HOST`
    /// removed and `$FLICK_REMOTE` set for a remote card.
    pub origin: Option<Origin>,
}

impl Job {
    /// The run for `run` pressed on `card`, or `None` when `run` is not a process.
    pub fn new(run: &Do, exe: &Path, origin: Origin, card: &str, press: u64) -> Option<Job> {
        let exe = exe.to_string_lossy().into_owned();
        let (argv, budget, label, origin) = match run {
            Do::Script { name, query } => {
                let mut argv = vec![exe, "script".into(), "run".into(), name.clone()];
                argv.extend(query.iter().cloned());
                (argv, FLICK_BUDGET, format!("script {name}"), Some(origin))
            }
            Do::Flick(words) => {
                let argv = std::iter::once(exe).chain(words.iter().cloned()).collect();
                (argv, FLICK_BUDGET, format!("flick {}", words.join(" ")), Some(origin))
            }
            Do::Shell(cmd) => {
                let argv = ["/bin/sh", "-c", cmd].map(String::from).to_vec();
                (argv, SHELL_BUDGET, "command".into(), None)
            }
            Do::OpenUrl(_) | Do::OpenApp(_) | Do::Copy(_) | Do::Dismiss => return None,
        };
        let label = clip(&label, LABEL_MAX);
        Some(Job { card: card.into(), press, argv, budget, label, home: origin.is_none(), origin })
    }
}

/// How a run ended: the card's muted line, or its error line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ran {
    Ok(String),
    Failed(String),
}

/// A finished run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    pub card: String,
    pub press: u64,
    pub ran: Ran,
}

/// Run `job`, killed with its process group after `job.budget`.
pub fn exec(job: &Job) -> Ran {
    let Some((program, args)) = job.argv.split_first() else {
        return Ran::Failed(format!("{}: nothing to run", job.label));
    };
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Its own process group, so a timeout kills what the shell started too.
    cmd.process_group(0);
    if job.home
        && let Some(home) = dirs::home_dir()
    {
        cmd.current_dir(home);
    }
    if let Some(origin) = job.origin {
        cmd.env_remove("FLICK_HOST");
        match origin {
            Origin::Remote => cmd.env("FLICK_REMOTE", "1"),
            Origin::Local => cmd.env_remove("FLICK_REMOTE"),
        };
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ran::Failed(format!("{}: {e}", job.label)),
    };
    let out = reader(child.stdout.take());
    let err = reader(child.stderr.take());
    let deadline = Instant::now() + job.budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => {
                kill(&mut child);
                return Ran::Failed(format!("{} took over {} s; stopped", job.label, job.budget.as_secs()));
            }
            Err(e) => return Ran::Failed(format!("{}: {e}", job.label)),
        }
    };
    // A background grandchild may keep a pipe open; never wait on it for long.
    let wait = Duration::from_secs(1);
    let (out, err) = (out.recv_timeout(wait).unwrap_or_default(), err.recv_timeout(wait).unwrap_or_default());
    outcome(&job.label, status.code(), &out, &err)
}

/// Kill `child`'s process group, then the child, and reap it.
fn kill(child: &mut Child) {
    let group = format!("-{}", child.id());
    let _ = Command::new("/bin/kill").args(["-KILL", "--", &group]).status();
    let _ = child.kill();
    let _ = child.wait();
}

/// The tail of `stream`, read to its end on a thread.
fn reader(stream: Option<impl Read + Send + 'static>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    if let Some(mut stream) = stream {
        thread::spawn(move || {
            let _ = tx.send(tail(&mut stream));
        });
    }
    rx
}

/// The last `TAIL` bytes of `r`, as text.
fn tail(r: &mut impl Read) -> String {
    let (mut kept, mut buf) = (Vec::new(), [0u8; 1024]);
    while let Ok(n) = r.read(&mut buf) {
        if n == 0 {
            break;
        }
        kept.extend_from_slice(&buf[..n]);
        if kept.len() > TAIL {
            kept.drain(..kept.len() - TAIL);
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// The card's line for a run of `label` that exited with `code` (`None`: a signal).
pub fn outcome(label: &str, code: Option<i32>, out: &str, err: &str) -> Ran {
    match code {
        Some(0) => Ran::Ok(last_line(out).unwrap_or_else(|| format!("Ran {label}"))),
        Some(n) => Ran::Failed(match last_line(err).or_else(|| last_line(out)) {
            Some(line) => format!("{label} failed (exit {n}): {line}"),
            None => format!("{label} failed (exit {n})"),
        }),
        None => Ran::Failed(format!("{label} was killed by a signal")),
    }
}

/// `s` cut to `max` chars with `…`.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.into();
    }
    s.chars().take(max - 1).chain(['…']).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(cmd: &str, budget: Duration) -> Job {
        let job = Job::new(&Do::Shell(cmd.into()), Path::new("/x"), Origin::Local, "c1", 1);
        Job { budget, ..job.unwrap() }
    }

    #[test]
    fn jobs_self_exec_flick_and_run_shell_through_sh() {
        let exe = Path::new("/Applications/Flick.app/Contents/MacOS/Flick");
        let script = Do::Script { name: "deploy".into(), query: Some("prod".into()) };
        let job = Job::new(&script, exe, Origin::Remote, "c1", 4).unwrap();
        assert_eq!(job.argv, [exe.to_str().unwrap(), "script", "run", "deploy", "prod"]);
        assert_eq!((job.budget, job.label.as_str(), job.home, job.origin), (FLICK_BUDGET, "script deploy", false, Some(Origin::Remote)));
        assert_eq!((job.card.as_str(), job.press), ("c1", 4));
        let bare = Do::Script { name: "lock".into(), query: None };
        assert_eq!(Job::new(&bare, exe, Origin::Local, "c", 1).unwrap().argv.len(), 4);
        let words = Do::Flick(["task", "start", "x"].map(String::from).to_vec());
        let job = Job::new(&words, exe, Origin::Local, "c", 1).unwrap();
        assert_eq!(job.argv[1..], ["task", "start", "x"]);
        assert_eq!(job.label, "flick task start x");
        let sh = Job::new(&Do::Shell("make deploy".into()), exe, Origin::Remote, "c", 1).unwrap();
        assert_eq!(sh.argv, ["/bin/sh", "-c", "make deploy"]);
        assert_eq!((sh.budget, sh.label.as_str(), sh.home, sh.origin), (SHELL_BUDGET, "command", true, None));
        for other in [Do::OpenUrl("https://x".into()), Do::OpenApp("Safari".into()), Do::Copy("t".into()), Do::Dismiss] {
            assert_eq!(Job::new(&other, exe, Origin::Local, "c", 1), None);
        }
        let long = Do::Flick(vec!["x".repeat(100)]);
        let label = Job::new(&long, exe, Origin::Local, "c", 1).unwrap().label;
        assert_eq!(label.chars().count(), LABEL_MAX);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn outcomes_give_the_last_line() {
        assert_eq!(outcome("command", Some(0), "a\nall good\n", ""), Ran::Ok("all good".into()));
        assert_eq!(outcome("command", Some(0), "", "noise"), Ran::Ok("Ran command".into()));
        assert_eq!(outcome("command", Some(3), "out", "bad thing\n"), Ran::Failed("command failed (exit 3): bad thing".into()));
        assert_eq!(outcome("command", Some(1), "only out", ""), Ran::Failed("command failed (exit 1): only out".into()));
        assert_eq!(outcome("command", Some(1), "", ""), Ran::Failed("command failed (exit 1)".into()));
        assert_eq!(outcome("command", None, "", ""), Ran::Failed("command was killed by a signal".into()));
    }

    #[test]
    fn exec_runs_in_home_and_reports_exits() {
        let budget = Duration::from_secs(10);
        let home = dirs::home_dir().unwrap();
        assert_eq!(exec(&shell("pwd", budget)), Ran::Ok(home.to_string_lossy().into_owned()));
        assert_eq!(exec(&shell("echo nope >&2; exit 4", budget)), Ran::Failed("command failed (exit 4): nope".into()));
        assert_eq!(exec(&shell("kill -9 $$", budget)), Ran::Failed("command was killed by a signal".into()));
        // Only the tail of a long output is kept.
        assert_eq!(exec(&shell("yes x | head -c 100000; echo; echo last", budget)), Ran::Ok("last".into()));
        // No stdin: a read sees end of file at once.
        assert_eq!(exec(&shell("read -r x || echo eof", budget)), Ran::Ok("eof".into()));
    }

    #[test]
    fn exec_sets_the_flick_environment_for_self_exec() {
        let budget = Duration::from_secs(10);
        let env = |origin| Job {
            argv: ["/bin/sh", "-c", "echo \"${FLICK_HOST:-none} ${FLICK_REMOTE:-local}\""].map(String::from).to_vec(),
            origin,
            ..shell("", budget)
        };
        assert_eq!(exec(&env(Some(Origin::Remote))), Ran::Ok("none 1".into()));
        assert_eq!(exec(&env(Some(Origin::Local))), Ran::Ok("none local".into()));
    }

    #[test]
    fn exec_kills_the_group_at_the_budget_and_reports_spawn_failures() {
        let started = Instant::now();
        // The shell's child sleeps too; killing only the shell would leave it holding stdout.
        let slow = shell("sleep 5 & sleep 5; wait", Duration::from_millis(200));
        assert_eq!(exec(&slow), Ran::Failed("command took over 0 s; stopped".into()));
        assert!(started.elapsed() < Duration::from_secs(4));
        let missing = Job { argv: vec!["/nonexistent/flick".into()], label: "script x".into(), ..shell("", started.elapsed()) };
        let Ran::Failed(e) = exec(&missing) else { panic!("spawned") };
        assert!(e.starts_with("script x: "), "{e}");
        let empty = Job { argv: vec![], ..missing };
        assert_eq!(exec(&empty), Ran::Failed("script x: nothing to run".into()));
    }
}
