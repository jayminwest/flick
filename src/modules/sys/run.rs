//! The real I/O behind `io::Hooks`: a child process with a time budget, a TCP connect with
//! a timeout, and this user's uid. Only background threads call these (and the first
//! `sys snapshot`, which waits on one with the same budget).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

/// A finished child process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exit {
    /// `None` when a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Run `argv` (no shell) with stdin closed; kill it when `budget` runs out.
pub fn run(argv: &[String], budget: Duration) -> Result<Exit, String> {
    run_with(argv, None, budget)
}

/// `run` with `input` written to the child's stdin, then stdin closed (the ssh fleet probe).
pub fn run_input(argv: &[String], input: &str, budget: Duration) -> Result<Exit, String> {
    run_with(argv, Some(input), budget)
}

fn run_with(argv: &[String], input: Option<&str>, budget: Duration) -> Result<Exit, String> {
    let (program, rest) = argv.split_first().ok_or("empty command")?;
    let mut child = Command::new(program)
        .args(rest)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    // Write on a thread too: a child that never reads must not block us past the budget.
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        let text = text.to_string();
        thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    // Read on threads so a full pipe never stalls the child while we wait for it.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut out = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut out);
            }
            out
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // A grandchild may hold the pipes open; leave the readers be.
                return Err(format!("timed out after {} s", budget.as_secs_f32()));
            }
            Err(e) => return Err(format!("{program}: {e}")),
        }
    };
    let (stdout, stderr) = (stdout.join().unwrap_or_default(), stderr.join().unwrap_or_default());
    Ok(Exit { code: status.code(), stdout, stderr })
}

/// Connect to `target` (`host:port`) within `budget`, trying each address it resolves to;
/// how long the successful connect took.
pub fn connect(target: &str, budget: Duration) -> Result<Duration, String> {
    let addrs: Vec<_> = target.to_socket_addrs().map_err(|e| format!("{target}: {e}"))?.collect();
    let mut last = format!("{target}: no address");
    for addr in addrs {
        let start = Instant::now();
        match TcpStream::connect_timeout(&addr, budget) {
            Ok(_) => return Ok(start.elapsed()),
            Err(e) => last = format!("{addr}: {e}"),
        }
    }
    Err(last)
}

/// This user's uid (`id -u`, once per process), for the `gui/<uid>` launchd domain.
pub fn uid() -> Option<u32> {
    static UID: OnceLock<Option<u32>> = OnceLock::new();
    *UID.get_or_init(|| {
        let argv = ["/usr/bin/id".to_string(), "-u".to_string()];
        run(&argv, Duration::from_secs(2)).ok()?.stdout.trim().parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn sh(script: &str) -> Vec<String> {
        vec!["/bin/sh".into(), "-c".into(), script.into()]
    }

    #[test]
    fn runs_a_child_and_keeps_its_output() {
        let exit = run(&sh("echo out; echo err >&2; exit 3"), Duration::from_secs(5)).unwrap();
        assert_eq!(exit, Exit { code: Some(3), stdout: "out\n".into(), stderr: "err\n".into() });
        let killed = run(&sh("kill -9 $$"), Duration::from_secs(5)).unwrap();
        assert_eq!(killed.code, None);
    }

    #[test]
    fn feeds_stdin_to_a_child() {
        let argv = vec!["/bin/sh".to_string(), "-s".into()];
        let exit = run_input(&argv, "echo from stdin; exit 4\n", Duration::from_secs(5)).unwrap();
        assert_eq!((exit.code, exit.stdout.as_str()), (Some(4), "from stdin\n"));
        // A child that never reads its stdin still ends at the budget.
        let err = run_input(&sh("exec sleep 5"), &"x".repeat(1 << 20), Duration::from_millis(100)).unwrap_err();
        assert_eq!(err, "timed out after 0.1 s");
    }

    #[test]
    fn kills_a_child_over_budget() {
        let start = Instant::now();
        let err = run(&sh("exec sleep 5"), Duration::from_millis(100)).unwrap_err();
        assert_eq!(err, "timed out after 0.1 s");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn reports_a_missing_program() {
        assert_eq!(run(&[], Duration::from_secs(1)).unwrap_err(), "empty command");
        let err = run(&["/nonexistent/flick-sys".into()], Duration::from_secs(1)).unwrap_err();
        assert!(err.starts_with("/nonexistent/flick-sys: No such file"), "{err}");
    }

    #[test]
    fn connects_to_a_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        assert!(connect(&addr, Duration::from_secs(1)).unwrap() < Duration::from_secs(1));
        // Port 0 never accepts; a freed ephemeral port could be taken by another test.
        let err = connect("127.0.0.1:0", Duration::from_secs(1)).unwrap_err();
        assert!(err.starts_with("127.0.0.1:0: "), "{err}");
        assert!(connect("no port", Duration::from_secs(1)).unwrap_err().starts_with("no port: "));
    }

    #[test]
    fn knows_the_uid() {
        assert!(uid().is_some());
        assert_eq!(uid(), uid());
    }
}
