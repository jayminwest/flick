//! One HTTP call through `/usr/bin/curl` (no HTTP crate: no binary cost, TLS for tailscale
//! serve's https endpoints for free). The argv holds only fixed flags and the URL; a
//! request body goes over curl's stdin (`--data-binary @-`), so prompt text never shows in
//! `ps`. `-q` (first) skips `~/.curlrc`, `--noproxy *` keeps a proxy from seeing the
//! text, `--proto =http,https` and no `-L` keep curl on the configured URL.
//!
//! `run` drives one call on the calling thread: spawn (through `Spawn`, which tests fake),
//! hand the child to the caller (cancel and the watchdog kill it), write and close stdin,
//! hand each stdout line to the caller, then reap. Buffers that held request or reply bytes
//! are zeroed before they are freed (best effort: `wipe`).

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

/// The one curl Flick runs: never a `PATH` lookup.
pub const CURL: &str = "/usr/bin/curl";
/// Seconds curl may take to connect.
pub const CONNECT_SECS: u64 = 5;
/// The most stderr kept from one call.
const STDERR_CAP: u64 = 4_096;

/// A running program.
pub trait Child: Send {
    /// Stop it now (SIGKILL). Its stdout then ends.
    fn kill(&mut self);
    /// Reap it: its exit code, `None` when a signal ended it.
    fn wait(&mut self) -> Option<i32>;
}

/// A started program with its pipes.
pub struct Spawned {
    pub stdin: Box<dyn Write + Send>,
    pub stdout: Box<dyn Read + Send>,
    pub stderr: Box<dyn Read + Send>,
    pub child: Box<dyn Child>,
}

/// Start an argv (program, then arguments) with all three pipes.
pub type Spawn = fn(&[String]) -> Result<Spawned, String>;

/// The child, shared by the reading thread and whoever stops it.
pub type ChildRef = Arc<Mutex<Box<dyn Child>>>;

pub fn lock(child: &ChildRef) -> MutexGuard<'_, Box<dyn Child>> {
    child.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One request.
pub struct Call {
    pub url: String,
    /// A JSON body to POST; `None`: GET.
    pub body: Option<Vec<u8>>,
    /// curl's `--max-time`, seconds.
    pub max_time: u64,
}

/// How a call ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    /// curl's exit code; `None` when killed.
    pub code: Option<i32>,
    pub stderr: String,
}

impl Ended {
    /// What went wrong, if curl failed: its `curl: (7) ...` line, else the code.
    pub fn failure(&self) -> Option<String> {
        let line = self.stderr.lines().map(str::trim).find(|l| !l.is_empty());
        let line = line.map(|l| l.strip_prefix("curl: ").unwrap_or(l).to_string());
        match self.code {
            Some(0) => None,
            Some(code) => Some(line.unwrap_or_else(|| format!("curl exited {code}"))),
            None => Some(line.unwrap_or_else(|| "curl was stopped".into())),
        }
    }
}

/// curl's argv for `call`. Nothing from the body is in it.
pub fn argv(call: &Call) -> Vec<String> {
    let (connect, max) = (CONNECT_SECS.to_string(), call.max_time.to_string());
    let mut argv = vec![CURL, "-q", "-sS", "-N", "--fail-with-body", "--noproxy", "*", "--proto", "=http,https"];
    argv.extend(["--connect-timeout", &connect, "--max-time", &max]);
    if call.body.is_some() {
        argv.extend(["-H", "Content-Type: application/json", "-H", "Accept: text/event-stream", "--data-binary", "@-"]);
    }
    argv.extend(["--url", &call.url]);
    argv.into_iter().map(str::to_string).collect()
}

/// Zero a buffer's whole allocation, then empty it. Best effort against a later read of
/// freed memory: copies made by earlier reallocations, curl and the kernel are out of reach.
pub fn wipe(buf: &mut Vec<u8>) {
    buf.clear();
    buf.resize(buf.capacity(), 0);
    std::hint::black_box(&buf);
    buf.clear();
}

/// `wipe` for a `String`.
pub fn wipe_string(s: &mut String) {
    let mut bytes = std::mem::take(s).into_bytes();
    wipe(&mut bytes);
}

/// Splits a byte stream into lines (`\n`; a trailing `\r` stays for the reader).
#[derive(Default)]
pub struct Lines {
    buf: Vec<u8>,
}

impl Lines {
    /// Add `bytes`; call `line` for each line they complete.
    pub fn push(&mut self, bytes: &[u8], line: &mut impl FnMut(&str)) {
        self.buf.extend_from_slice(bytes);
        let mut start = 0;
        while let Some(n) = self.buf[start..].iter().position(|b| *b == b'\n') {
            let mut text = String::from_utf8_lossy(&self.buf[start..start + n]).into_owned();
            line(&text);
            wipe_string(&mut text);
            start += n + 1;
        }
        if start > 0 {
            self.buf[..start].fill(0);
            self.buf.drain(..start);
        }
    }

    /// The stream ended: call `line` for an unfinished last line, then wipe.
    pub fn finish(mut self, line: &mut impl FnMut(&str)) {
        if !self.buf.is_empty() {
            let mut text = String::from_utf8_lossy(&self.buf).into_owned();
            line(&text);
            wipe_string(&mut text);
        }
        wipe(&mut self.buf);
    }
}

/// Run `call` to its end on this thread. `started` gets the child as soon as it runs;
/// `line` gets each stdout line. `Err`: curl did not start.
pub fn run(mut call: Call, spawn: Spawn, started: impl FnOnce(&ChildRef), mut line: impl FnMut(&str)) -> Result<Ended, String> {
    let spawned = spawn(&argv(&call));
    let Spawned { mut stdin, mut stdout, stderr, child } = match spawned {
        Ok(s) => s,
        Err(e) => {
            if let Some(body) = call.body.as_mut() {
                wipe(body);
            }
            return Err(e);
        }
    };
    let child: ChildRef = Arc::new(Mutex::new(child));
    started(&child);
    let errors = thread::Builder::new().name("llm-curl-stderr".into()).spawn(move || {
        let mut text = String::new();
        let _ = stderr.take(STDERR_CAP).read_to_string(&mut text);
        text
    });
    if let Some(mut body) = call.body.take() {
        // curl reads the whole body before the reply comes, so this cannot deadlock. A
        // failed write (curl gone) shows in its exit code.
        let _ = stdin.write_all(&body);
        wipe(&mut body);
    }
    drop(stdin);
    let mut lines = Lines::default();
    let mut chunk = vec![0u8; 8_192];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => lines.push(&chunk[..n], &mut line),
        }
    }
    wipe(&mut chunk);
    lines.finish(&mut line);
    let code = lock(&child).wait();
    let stderr = errors.ok().and_then(|t| t.join().ok()).unwrap_or_default();
    Ok(Ended { code, stderr })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn get(url: &str) -> Call {
        Call { url: url.into(), body: None, max_time: 3 }
    }

    #[test]
    fn argv_has_fixed_flags_and_the_url_only() {
        let want = "/usr/bin/curl -q -sS -N --fail-with-body --noproxy * --proto =http,https --connect-timeout 5 \
            --max-time 3 --url http://h/v1/models";
        assert_eq!(argv(&get("http://h/v1/models")).join(" "), want);
        let post = Call { url: "https://h/v1/chat/completions".into(), body: Some(b"{\"secret prompt\"}".to_vec()), max_time: 300 };
        let a = argv(&post);
        assert_eq!(a[1], "-q", "-q must come first or curl reads ~/.curlrc");
        assert!(a.join(" ").contains("--max-time 300 -H Content-Type: application/json -H Accept: text/event-stream"));
        assert_eq!(a[a.len() - 4..], ["--data-binary", "@-", "--url", "https://h/v1/chat/completions"]);
        assert!(!a.iter().any(|w| w.contains("secret")));
    }

    #[test]
    fn failures_name_curls_error() {
        let ended = |code, stderr: &str| Ended { code, stderr: stderr.into() }.failure();
        assert_eq!(ended(Some(0), "noise"), None);
        assert_eq!(ended(Some(7), "\ncurl: (7) Failed to connect\n").as_deref(), Some("(7) Failed to connect"));
        assert_eq!(ended(Some(28), "").as_deref(), Some("curl exited 28"));
        assert_eq!(ended(None, "").as_deref(), Some("curl was stopped"));
        assert_eq!(ended(None, "odd").as_deref(), Some("odd"));
    }

    #[test]
    fn wipe_zeroes_and_empties() {
        let mut v = b"secret".to_vec();
        v.reserve(10);
        let cap = v.capacity();
        wipe(&mut v);
        assert!(v.is_empty());
        assert_eq!(v.capacity(), cap);
        let mut s = String::from("secret");
        wipe_string(&mut s);
        assert!(s.is_empty());
    }

    #[test]
    fn lines_split_across_chunks() {
        let mut got = vec![];
        let mut lines = Lines::default();
        let mut add = |l: &str| got.push(l.to_string());
        lines.push(b"data: a\r\nda", &mut add);
        lines.push(b"ta: b\n\n", &mut add);
        lines.push(&[0xff, b'x'], &mut add);
        lines.finish(&mut add);
        assert_eq!(got, ["data: a\r", "data: b", "", "\u{fffd}x"]);
        let mut none = 0;
        Lines::default().finish(&mut |_| none += 1);
        assert_eq!(none, 0);
    }

    struct Fake(Option<i32>);

    impl Child for Fake {
        fn kill(&mut self) {
            self.0 = None;
        }

        fn wait(&mut self) -> Option<i32> {
            self.0
        }
    }

    #[test]
    fn a_killed_child_has_no_code() {
        let mut child = Fake(Some(0));
        child.kill();
        assert_eq!(child.wait(), None);
    }

    static SPAWNS: AtomicUsize = AtomicUsize::new(0);

    /// Echoes nothing it was sent: prints two lines and an unfinished one, exits 0.
    #[expect(clippy::unnecessary_wraps, reason = "a fake Spawn answers as the real one does")]
    fn spawn_ok(argv: &[String]) -> Result<Spawned, String> {
        assert_eq!(argv[0], CURL);
        SPAWNS.fetch_add(1, Ordering::Relaxed);
        Ok(Spawned {
            stdin: Box::new(std::io::sink()),
            stdout: Box::new(Cursor::new(b"one\ntwo\nthr".to_vec())),
            stderr: Box::new(Cursor::new(b"warn".to_vec())),
            child: Box::new(Fake(Some(0))),
        })
    }

    #[test]
    fn run_hands_over_the_child_and_every_line() {
        let mut got = vec![];
        let mut seen = false;
        let call = Call { body: Some(b"{}".to_vec()), ..get("http://h") };
        let ended = run(call, spawn_ok, |c| seen = lock(c).wait() == Some(0), |l| got.push(l.to_string())).unwrap();
        assert!(seen);
        assert_eq!(got, ["one", "two", "thr"]);
        assert_eq!(ended, Ended { code: Some(0), stderr: "warn".into() });
        assert!(SPAWNS.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn a_spawn_failure_is_an_error() {
        let fail: Spawn = |_| Err("curl not found".into());
        let call = Call { body: Some(b"{}".to_vec()), ..get("http://h") };
        let out = run(call, fail, |_| panic!("no child"), |_| panic!("no lines"));
        assert_eq!(out.unwrap_err(), "curl not found");
        assert_eq!(run(get("http://h"), fail, |_| {}, |_| {}).unwrap_err(), "curl not found");
    }
}
