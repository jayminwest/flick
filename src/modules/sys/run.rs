//! The real I/O behind `io::Hooks`: a child process with a time budget. Only background
//! threads call it.

use std::io::Read;
use std::process::{Command, Stdio};
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
    let (program, rest) = argv.split_first().ok_or("empty command")?;
    let mut child = Command::new(program)
        .args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

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


}
