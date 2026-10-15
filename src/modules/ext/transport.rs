//! One call to an extension: spawn its argv (no shell) in its own process group, write the
//! request to stdin, read one reply from stdout, wait for exit 0. Past the call's budget the
//! group is killed, and so it is when `stop` says a newer query came (newest wins). Only
//! the extension's worker thread calls this.
//!
//! The child gets `FLICK_EXT` (its name), `FLICK_SOCKET` (this Flick's control socket, for
//! `flick ...` calls back) and `FLICK_VERSION`, and `FLICK_HOST` removed so the CLI talks to
//! this Mac. Every error starts with `<name>: `; a failed exit gives the last stderr line.
//!
//! `flick` runs Flick's own CLI for an action's card `script` or `flick` `do`, the same way.
//! `raw` (`ext call`) sends a request text as given and returns the stdout unparsed.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::icons;
use super::proto::{self, Done, Op, Parsed, REPLY_MAX, Reply, Request, TEXT_MAX};
use super::settings::Spec;

/// Time a card `script` or `flick` request may take: `script run` only starts the script,
/// and a request answers in milliseconds, so this is generous.
pub const FLICK_BUDGET: Duration = Duration::from_secs(30);

/// How often the worker checks its child.
const POLL: Duration = Duration::from_millis(10);
/// Bytes of stderr kept, from the end: enough for the last line.
const TAIL: usize = 4096;
/// The error of a call killed because a newer query came; never shown.
pub const STOPPED: &str = "a newer query came; stopped";
/// How long to wait for the pipes after exit; a grandchild may hold them open.
const DRAIN: Duration = Duration::from_secs(1);

/// Run `req` on `spec` and parse its reply, its icon files checked. `socket` is
/// `FLICK_SOCKET`; `stop`, asked while the child runs, kills it when true.
pub fn call(spec: &Spec, req: &Request, socket: &Path, stop: &dyn Fn() -> bool) -> Result<Parsed, String> {
    let name = &spec.name;
    let out = exec(spec, req.to_json(), spec.budget(req.op()), socket, stop).map_err(|e| format!("{name}: {e}"))?;
    proto::parse(req.op(), &out).map(icons::check).map_err(|e| format!("{name}: {e}"))
}

/// Send `text` to `spec` as it is and return its stdout, trailing whitespace trimmed,
/// without parsing it. It gets the long budget of `act`, whatever op it names.
pub fn raw(spec: &Spec, text: &str, socket: &Path) -> Result<String, String> {
    let out = exec(spec, text.to_string(), spec.budget(Op::Act), socket, &|| false).map_err(|e| format!("{}: {e}", spec.name))?;
    Ok(String::from_utf8_lossy(&out).trim_end().to_string())
}

/// The child's stdout, or why the call failed (without the name).
fn exec(spec: &Spec, json: String, budget: Duration, socket: &Path, stop: &dyn Fn() -> bool) -> Result<Vec<u8>, String> {
    let (program, args) = spec.argv.split_first().ok_or("nothing to run")?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("FLICK_EXT", &spec.name)
        .env("FLICK_SOCKET", socket)
        .env("FLICK_VERSION", env!("CARGO_PKG_VERSION"))
        .env_remove("FLICK_HOST")
        // Its own process group, so a timeout kills what it started too.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    // Write on a thread: a child that never reads must not hold us past the budget.
    if let Some(mut stdin) = child.stdin.take() {
        thread::spawn(move || {
            let _ = stdin.write_all(json.as_bytes()).and_then(|()| stdin.write_all(b"\n"));
        });
    }
    finish(child, budget, stop)
}

/// Run Flick's CLI `exe` with `words`, without stdin, as a local request to this Mac
/// (`FLICK_HOST` and `FLICK_REMOTE` removed). Its status is the last line it printed.
pub fn flick(exe: &Path, words: &[String]) -> Result<Parsed, String> {
    let child = Command::new(exe)
        .args(words)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("FLICK_HOST")
        .env_remove("FLICK_REMOTE")
        .process_group(0)
        .spawn()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let out = finish(child, FLICK_BUDGET, &|| false)?;
    let status = last_line(&out).filter(|l| !l.chars().any(proto::hidden));
    let status = status.map(|l| l.chars().take(TEXT_MAX).collect());
    Ok(Parsed { reply: Reply::Done(Done { status, refresh: false }), warnings: vec![] })
}

/// The last line of `bytes` that is not blank, trimmed.
fn last_line(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    text.lines().rev().map(str::trim).find(|l| !l.is_empty()).map(String::from)
}

/// Wait for `child` to exit within `budget` and before `stop` is true, else kill its group.
/// Its stdout on exit 0, else the last stderr line.
fn finish(mut child: Child, budget: Duration, stop: &dyn Fn() -> bool) -> Result<Vec<u8>, String> {
    let out = reader(child.stdout.take(), Keep::Head(REPLY_MAX + 1));
    let err = reader(child.stderr.take(), Keep::Tail(TAIL));
    let deadline = Instant::now() + budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if stop() => {
                kill(&mut child);
                return Err(STOPPED.into());
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) => {
                kill(&mut child);
                return Err(format!("no reply in {} ms; stopped", budget.as_millis()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let (out, err) = (out.recv_timeout(DRAIN).unwrap_or_default(), err.recv_timeout(DRAIN));
    let last = || last_line(&err.clone().unwrap_or_default());
    match status.code() {
        Some(0) if out.len() > REPLY_MAX => Err(format!("reply over {REPLY_MAX} bytes")),
        Some(0) => Ok(out),
        Some(n) => Err(last().unwrap_or_else(|| format!("exit {n}"))),
        None => Err(last().unwrap_or_else(|| "killed by a signal".into())),
    }
}

/// Kill `child`'s process group, then the child, and reap it (as `message::local` does).
fn kill(child: &mut Child) {
    let group = format!("-{}", child.id());
    let _ = Command::new("/bin/kill").args(["-KILL", "--", &group]).status();
    let _ = child.kill();
    let _ = child.wait();
}

/// Which bytes of a stream to keep. The stream is read to its end either way, so a full
/// pipe never stalls the child.
#[derive(Clone, Copy)]
enum Keep {
    Head(usize),
    Tail(usize),
}

/// The kept bytes of `stream`, read to its end on a thread.
fn reader(stream: Option<impl Read + Send + 'static>, keep: Keep) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    if let Some(mut stream) = stream {
        thread::spawn(move || {
            let (mut kept, mut buf) = (Vec::new(), [0u8; 8192]);
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                kept.extend_from_slice(&buf[..n]);
                match keep {
                    Keep::Head(max) => kept.truncate(max),
                    Keep::Tail(max) => drop(kept.drain(..kept.len().saturating_sub(max))),
                }
            }
            let _ = tx.send(kept);
        });
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ext::proto::{Done, Reply};
    use crate::modules::ext::settings::LONG;

    pub const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ext-echo.sh");

    fn spec(mode: &[&str], timeout: Duration) -> Spec {
        let argv = std::iter::once(FIXTURE).chain(mode.iter().copied()).map(String::from);
        Spec { name: "echo".into(), argv: argv.collect(), timeout, hotkeys: vec![], keywords: vec![] }
    }

    fn run(mode: &[&str]) -> Result<Parsed, String> {
        call(&spec(mode, Duration::from_secs(10)), &Request::Root, Path::new("/tmp/flick.sock"), &|| false)
    }

    #[test]
    fn a_call_writes_the_request_and_sets_the_environment() {
        let p = run(&["echo"]).unwrap();
        let Reply::Items(items) = p.reply else { panic!("{:?}", p.reply) };
        assert_eq!(items[0].key, "root");
        assert_eq!(items[0].title, format!("echo {}", env!("CARGO_PKG_VERSION")));
        assert_eq!(items[0].subtitle, "/tmp/flick.sock");
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        let act = Request::Act { view: None, item: "a".into(), action: "open".into() };
        let p = call(&spec(&["echo"], Duration::from_secs(10)), &act, Path::new("/s"), &|| false).unwrap();
        assert_eq!(p.reply, Reply::Done(Done { status: Some("act done".into()), refresh: false }));
    }

    #[test]
    fn failures_name_the_extension() {
        assert_eq!(run(&["fail"]).unwrap_err(), "echo: bad token");
        assert_eq!(run(&["exit"]).unwrap_err(), "echo: exit 3");
        assert_eq!(run(&["signal"]).unwrap_err(), "echo: killed by a signal");
        assert_eq!(run(&["error"]).unwrap_err(), "echo: no such view");
        assert!(run(&["garbage"]).unwrap_err().starts_with("echo: reply is not JSON"));
        assert_eq!(run(&["huge"]).unwrap_err(), format!("echo: reply over {REPLY_MAX} bytes"));
        let missing = Spec { argv: vec!["/nonexistent/ext".into()], ..spec(&[], LONG) };
        let e = call(&missing, &Request::Root, Path::new("/s"), &|| false).unwrap_err();
        assert!(e.starts_with("echo: /nonexistent/ext: "), "{e}");
        let empty = Spec { argv: vec![], ..missing };
        assert_eq!(call(&empty, &Request::Root, Path::new("/s"), &|| false).unwrap_err(), "echo: nothing to run");
    }

    #[test]
    fn flick_runs_the_cli_and_its_last_line_is_the_status() {
        let words = |w: &[&str]| w.iter().map(ToString::to_string).collect::<Vec<_>>();
        let done = |status: Option<&str>| Reply::Done(Done { status: status.map(String::from), refresh: false });
        let p = flick(Path::new("/bin/echo"), &words(&["task", "ls"])).unwrap();
        assert_eq!(p.reply, done(Some("task ls")));
        assert_eq!(flick(Path::new("/usr/bin/true"), &[]).unwrap().reply, done(None));
        assert_eq!(flick(Path::new(FIXTURE), &words(&["fail"])).unwrap_err(), "bad token");
        let e = flick(Path::new("/nonexistent/flick"), &[]).unwrap_err();
        assert!(e.starts_with("/nonexistent/flick: "), "{e}");
    }

    #[test]
    fn a_raw_call_sends_the_text_as_given_and_returns_the_stdout() {
        let echo = spec(&["echo"], Duration::from_secs(10));
        let out = raw(&echo, r#"{"op":"root"}"#, Path::new("/s")).unwrap();
        let want = format!(r#"{{"v":1,"status":"root done","items":[{{"key":"root","title":"echo {}","subtitle":"/s"}}]}}"#, env!("CARGO_PKG_VERSION"));
        assert_eq!(out, want);
        // Not parsed: an error reply or text that is not JSON comes back as it is.
        assert_eq!(raw(&spec(&["error"], LONG), "{}", Path::new("/s")).unwrap(), r#"{"error":"no such view"}"#);
        assert_eq!(raw(&spec(&["garbage"], LONG), "x", Path::new("/s")).unwrap(), "not json");
        assert_eq!(raw(&spec(&["fail"], LONG), "{}", Path::new("/s")).unwrap_err(), "echo: bad token");
        // Any op gets the long budget of `act`.
        let dir = crate::core::scratch::Scratch::new("ext-raw");
        let pids = dir.join("pids");
        let slow = spec(&["sleep", "1", pids.to_str().unwrap()], Duration::from_millis(100));
        assert_eq!(raw(&slow, r#"{"op":"root"}"#, Path::new("/s")), Ok(String::new()));
    }

    #[test]
    fn a_call_is_killed_when_a_newer_query_stops_it() {
        let dir = crate::core::scratch::Scratch::new("ext-stop");
        let pids = dir.join("pids");
        let slow = spec(&["sleep", "30", pids.to_str().unwrap()], Duration::from_secs(30));
        let started = Instant::now();
        let query = Request::Query { view: "v".into(), query: "q".into() };
        let e = call(&slow, &query, Path::new("/s"), &|| pids.exists()).unwrap_err();
        assert_eq!(e, format!("echo: {STOPPED}"));
        assert!(started.elapsed() < Duration::from_secs(15), "{:?}", started.elapsed());
    }

    #[test]
    fn a_child_that_does_not_read_stdin_still_answers() {
        assert!(run(&["deaf"]).is_ok());
    }

    #[test]
    fn a_slow_extension_is_killed_with_its_group_at_the_timeout() {
        let dir = crate::core::scratch::Scratch::new("ext-slow");
        let pids = dir.join("pids");
        let slow = spec(&["sleep", "30", pids.to_str().unwrap()], Duration::from_millis(1000));
        let started = Instant::now();
        let e = call(&slow, &Request::Root, Path::new("/s"), &|| false).unwrap_err();
        assert_eq!(e, "echo: no reply in 1000 ms; stopped");
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        // The script and the sleep it started are both gone (launchd reaps the orphan).
        let text = std::fs::read_to_string(&pids).unwrap();
        assert_eq!(text.split_whitespace().count(), 2, "{text}");
        let alive = |pid: &str| {
            let status = Command::new("/bin/kill").args(["-0", pid]).stderr(Stdio::null()).status();
            status.unwrap().success()
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while text.split_whitespace().any(alive) && Instant::now() < deadline {
            thread::sleep(POLL);
        }
        assert!(!text.split_whitespace().any(alive), "still running: {text}");
    }
}
