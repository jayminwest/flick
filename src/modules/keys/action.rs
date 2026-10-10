//! What a chord edge runs: an HTTP request or a shell command, on one FIFO worker thread
//! per module so an `up` never overtakes its `down`, and never on the main or tap thread;
//! or a Flick control request (`flick`), which the module hands to its main-queue hook.

use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Child, Command};
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use super::words;
use crate::core::card::{Do, Origin};

/// Connect, write and read limit for one HTTP action, so a dead service never holds the
/// worker for long.
const TIMEOUT: Duration = Duration::from_secs(2);

/// `on_down` / `on_up` as written: `{ http = "POST http://..." }`, `{ shell = "..." }` or
/// `{ flick = "<module> <verb> [args]" }`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Spec {
    Http(String),
    Shell(String),
    Flick(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Http(Request),
    /// Run with `/bin/sh -c`.
    Shell(String),
    /// A local control request, `["<module>", "<verb>", args...]` (split by `words::split`).
    Flick(Vec<String>),
}

/// An `http://` request with an empty body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    method: String,
    host: String,
    port: u16,
    /// Path and query, starting with `/`.
    path: String,
}

impl Action {
    /// Checks `spec`: `http` takes `[METHOD ]http://host[:port][/path]` (method defaults to
    /// POST; no https), `shell` a non-empty command, `flick` at least one word.
    pub fn parse(spec: Spec) -> Result<Action, String> {
        match spec {
            Spec::Http(text) => Request::parse(&text).map(Action::Http),
            Spec::Shell(cmd) if cmd.trim().is_empty() => Err("shell: empty command".into()),
            Spec::Shell(cmd) => Ok(Action::Shell(cmd)),
            Spec::Flick(text) => {
                let bad = |e: String| format!("flick \"{text}\": {}", e.trim_start_matches("flick: "));
                let words = words::split(&text).map_err(bad)?;
                // The same word policy as a card's flick action: a module first, no client
                // flags (--json, --remote, --host, --stdin), no events stream.
                Do::Flick(words.clone()).check(Origin::Local).map_err(bad)?;
                Ok(Action::Flick(words))
            }
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Http(r) => write!(f, "{} http://{}{}", r.method, r.authority(), r.path),
            Action::Shell(cmd) => write!(f, "shell {cmd}"),
            Action::Flick(w) => write!(f, "flick {}", words::join(w)),
        }
    }
}

impl Request {
    fn parse(text: &str) -> Result<Request, String> {
        let bad = |why: &str| format!("http \"{text}\": {why}");
        let (method, url) = match text.trim().split_once(' ') {
            Some((m, url)) => (m, url.trim()),
            None => ("POST", text.trim()),
        };
        if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(bad("method must be an uppercase word such as POST"));
        }
        let Some(rest) = url.strip_prefix("http://") else {
            return Err(bad("only http:// URLs are supported"));
        };
        if rest.contains(char::is_whitespace) {
            return Err(bad("URL contains a space"));
        }
        let rest = rest.split('#').next().unwrap_or_default();
        let (authority, path) = match rest.find(['/', '?']) {
            Some(i) => (&rest[..i], rest[i..].to_string()),
            None => (rest, "/".to_string()),
        };
        let path = if path.starts_with('?') { format!("/{path}") } else { path };
        let (host, port) = match authority.strip_prefix('[') {
            Some(v6) => v6.split_once(']').map(|(h, p)| (h, p.strip_prefix(':'))),
            None => Some(match authority.split_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (authority, None),
            }),
        }
        .ok_or_else(|| bad("unclosed [ in host"))?;
        if host.is_empty() {
            return Err(bad("missing host"));
        }
        let port = match port {
            Some(p) => p.parse().map_err(|_| bad("bad port"))?,
            None => 80,
        };
        Ok(Request { method: method.into(), host: host.into(), port, path })
    }

    /// `host[:port]`, as the Host header and the URL show it.
    fn authority(&self) -> String {
        let host =
            if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == 80 { host } else { format!("{host}:{}", self.port) }
    }

    /// Send the request and read the status line. `Ok("HTTP <code>")` for a status below
    /// 400; `Err` for a 4xx/5xx status, no connection or a timeout.
    pub fn send(&self) -> Result<String, String> {
        let addrs = (self.host.as_str(), self.port).to_socket_addrs().map_err(|e| e.to_string())?;
        let mut last = format!("{}: no address", self.host);
        let mut stream = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, TIMEOUT) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last = format!("{addr}: {e}"),
            }
        }
        let mut stream = stream.ok_or(last)?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
        let length = if matches!(self.method.as_str(), "GET" | "HEAD") {
            ""
        } else {
            "Content-Length: 0\r\n"
        };
        let head = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: flick\r\n{length}Connection: close\r\n\r\n",
            self.method,
            self.path,
            self.authority()
        );
        stream.write_all(head.as_bytes()).map_err(|e| e.to_string())?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
        let line = line.trim_end();
        match line.split(' ').nth(1).and_then(|c| c.parse::<u16>().ok()) {
            Some(code) if line.starts_with("HTTP/") && code < 400 => Ok(format!("HTTP {code}")),
            Some(_) if line.starts_with("HTTP/") => Err(line.to_string()),
            _ => Err(format!("bad response \"{line}\"")),
        }
    }
}

/// One action to run for chord `chord`'s `down` or up edge.
#[derive(Clone, Debug)]
pub struct Job {
    pub chord: String,
    pub down: bool,
    pub action: Action,
}

impl Job {
    fn state(&self) -> &'static str {
        if self.down { "down" } else { "up" }
    }

    /// Run the action. HTTP waits for the status line; a shell command is started with
    /// `FLICK_CHORD` and `FLICK_CHORD_STATE` set and reaped on its own thread, so a slow
    /// command never holds the queue.
    fn run(&self) -> Result<String, String> {
        match &self.action {
            Action::Http(req) => req.send(),
            // `Keys::fire` hands these to the main-queue hook; they never reach the worker.
            Action::Flick(_) => Err("flick actions run on the main queue".into()),
            Action::Shell(cmd) => {
                let mut child = self.spawn(cmd).map_err(|e| e.to_string())?;
                let pid = child.id();
                let (chord, state) = (self.chord.clone(), self.state());
                let reaper = thread::Builder::new().name("flick-keys-reap".into());
                reaper
                    .spawn(move || match child.wait() {
                        Ok(s) if s.success() => {}
                        Ok(s) => eprintln!("flick: keys: {chord} {state}: shell {s}"),
                        Err(e) => eprintln!("flick: keys: {chord} {state}: shell: {e}"),
                    })
                    .map_err(|e| e.to_string())?;
                Ok(format!("shell pid {pid}"))
            }
        }
    }

    fn spawn(&self, cmd: &str) -> std::io::Result<Child> {
        Command::new("/bin/sh")
            .args(["-c", cmd])
            .env("FLICK_CHORD", &self.chord)
            .env("FLICK_CHORD_STATE", self.state())
            .spawn()
    }
}

/// The module's action queue. The thread starts at the first job, blocks on the channel
/// (no polling) and ends when the module, and so the sender, is dropped.
#[derive(Default)]
pub struct Worker {
    tx: Option<Sender<Job>>,
}

impl Worker {
    /// No thread started yet.
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        self.tx.is_none()
    }

    /// Queue `job` behind every earlier one.
    pub fn send(&mut self, job: Job) -> Result<(), String> {
        let job = match &self.tx {
            Some(tx) => match tx.send(job) {
                Ok(()) => return Ok(()),
                // The thread is gone (it never panics, but be safe): start a new one.
                Err(e) => e.0,
            },
            None => job,
        };
        let (tx, rx) = channel::<Job>();
        thread::Builder::new()
            .name("flick-keys".into())
            .spawn(move || {
                for job in rx {
                    let state = job.state();
                    match job.run() {
                        Ok(done) => eprintln!("flick: keys: {} {state}: {done}", job.chord),
                        Err(e) => eprintln!("flick: keys: {} {state}: {} failed: {e}", job.chord, job.action),
                    }
                }
            })
            .map_err(|e| format!("keys: cannot start the action thread: {e}"))?;
        tx.send(job).map_err(|_| "keys: action thread stopped".to_string())?;
        self.tx = Some(tx);
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    fn http(text: &str) -> Result<Request, String> {
        match Action::parse(Spec::Http(text.into()))? {
            Action::Http(r) => Ok(r),
            _ => unreachable!(),
        }
    }

    #[test]
    fn parses_http_actions() {
        let r = http("POST http://localhost:8600/pipeline/listen/start").unwrap();
        assert_eq!((r.method.as_str(), r.host.as_str(), r.port), ("POST", "localhost", 8600));
        assert_eq!(r.path, "/pipeline/listen/start");
        let shown = Action::Http(r).to_string();
        assert_eq!(shown, "POST http://localhost:8600/pipeline/listen/start");
        assert_eq!(Action::Http(http("http://h").unwrap()).to_string(), "POST http://h/");
        let r = http("GET http://[::1]:9/a?b=1#frag").unwrap();
        assert_eq!((r.host.as_str(), r.port, r.path.as_str()), ("::1", 9, "/a?b=1"));
        assert_eq!(r.authority(), "[::1]:9");
        assert_eq!(http("http://h?q").unwrap().path, "/?q");
        assert_eq!(http("http://[::1]").unwrap().authority(), "[::1]");
        for (bad, why) in [
            ("https://h/", "only http:// URLs"),
            ("post http://h/", "method must be"),
            ("POST  http://h/a b", "space"),
            ("http://:80/", "missing host"),
            ("http://h:x/", "bad port"),
            ("http://[::1/", "unclosed"),
        ] {
            let err = http(bad).unwrap_err();
            assert!(err.starts_with(&format!("http \"{bad}\": ")) && err.contains(why), "{err}");
        }
        assert_eq!(Action::parse(Spec::Shell(" ".into())).unwrap_err(), "shell: empty command");
        assert_eq!(Action::parse(Spec::Shell("true".into())).unwrap().to_string(), "shell true");
    }

    #[test]
    fn parses_flick_actions() {
        let flick = Action::parse(Spec::Flick(" dictation  start ".into())).unwrap();
        assert_eq!(flick, Action::Flick(vec!["dictation".into(), "start".into()]));
        assert_eq!(flick.to_string(), "flick dictation start");
        let post = Action::parse(Spec::Flick("message post 'hi there'".into())).unwrap();
        assert_eq!(post.to_string(), "flick message post 'hi there'");
        let err = Action::parse(Spec::Flick(" ".into())).unwrap_err();
        assert_eq!(err, "flick \" \": empty request");
        let refused = |text: &str| Action::parse(Spec::Flick(text.into())).unwrap_err();
        assert_eq!(refused("--host x keys list"), "flick \"--host x keys list\": the first word must be a module");
        assert_eq!(refused("task ls --json"), "flick \"task ls --json\": --json is not allowed in an action");
        assert_eq!(refused("events"), "flick \"events\": events is a stream, not an action");
        let job = Job { chord: "d".into(), down: true, action: flick };
        assert_eq!(job.run().unwrap_err(), "flick actions run on the main queue");
    }

    /// A listener on an ephemeral port; `serve` answers each request with `status` and
    /// returns the request heads in arrival order.
    pub fn listener() -> (TcpListener, u16) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        (l, port)
    }

    pub fn serve(l: &TcpListener, n: usize, status: &str) -> Vec<String> {
        (0..n)
            .map(|_| {
                let (mut s, _) = l.accept().unwrap();
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut head = Vec::new();
                let mut byte = [0];
                while !head.ends_with(b"\r\n\r\n") && s.read(&mut byte).unwrap() == 1 {
                    head.push(byte[0]);
                }
                s.write_all(format!("HTTP/1.1 {status}\r\n\r\n").as_bytes()).unwrap();
                String::from_utf8(head).unwrap()
            })
            .collect()
    }

    #[test]
    fn sends_an_empty_post_and_reads_the_status() {
        let (l, port) = listener();
        let req = http(&format!("POST http://127.0.0.1:{port}/start")).unwrap();
        let t = thread::spawn(move || {
            let mut heads = serve(&l, 2, "204 No Content");
            heads.extend(serve(&l, 1, "500 Internal Server Error"));
            heads.extend(serve(&l, 1, "garbage"));
            heads
        });
        assert_eq!(req.send().unwrap(), "HTTP 204");
        let get = http(&format!("GET http://127.0.0.1:{port}/")).unwrap();
        assert_eq!(get.send().unwrap(), "HTTP 204");
        assert_eq!(req.send().unwrap_err(), "HTTP/1.1 500 Internal Server Error");
        assert_eq!(req.send().unwrap_err(), "bad response \"HTTP/1.1 garbage\"");
        let heads = t.join().unwrap();
        assert_eq!(
            heads[0],
            format!(
                "POST /start HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUser-Agent: flick\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
            )
        );
        assert!(!heads[1].contains("Content-Length"), "{}", heads[1]);
    }

    #[test]
    fn a_closed_port_is_an_error() {
        let (l, port) = listener();
        drop(l);
        // An IP literal, so no resolver is involved: the error names the refused address.
        let err = http(&format!("http://127.0.0.1:{port}/")).unwrap().send().unwrap_err();
        assert!(err.starts_with(&format!("127.0.0.1:{port}: ")), "{err}");
    }

    /// A unique temp file path for a shell action to write.
    pub fn temp(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("flk-{}-keys-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// Wait up to 5 s for `path` to hold a full line.
    pub fn read_when_written(path: &std::path::Path) -> String {
        for _ in 0..500 {
            if let Ok(text) = std::fs::read_to_string(path)
                && text.ends_with('\n')
            {
                return text;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("{} was never written", path.display());
    }

    #[test]
    fn shell_gets_the_chord_and_state() {
        let out = temp("shell");
        let cmd = format!("echo \"$FLICK_CHORD $FLICK_CHORD_STATE\" > '{}'", out.display());
        let job = Job { chord: "ptt".into(), down: false, action: Action::Shell(cmd.clone()) };
        let mut child = job.spawn(&cmd).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "ptt up\n");
        // `run` starts it and leaves the exit to the reaper, failures included.
        let fail = Job { action: Action::Shell("exit 3".into()), ..job.clone() };
        assert!(fail.run().unwrap().starts_with("shell pid "));
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn worker_runs_jobs_in_order() {
        let (l, port) = listener();
        let action = |path: &str| {
            Action::parse(Spec::Http(format!("http://127.0.0.1:{port}/{path}"))).unwrap()
        };
        let mut worker = Worker::default();
        for (down, path) in [(true, "start"), (false, "stop"), (true, "start")] {
            worker.send(Job { chord: "ptt".into(), down, action: action(path) }).unwrap();
        }
        let heads = serve(&l, 3, "200 OK");
        let lines: Vec<_> = heads.iter().map(|h| h.lines().next().unwrap()).collect();
        assert_eq!(lines, ["POST /start HTTP/1.1", "POST /stop HTTP/1.1", "POST /start HTTP/1.1"]);
        // A worker whose thread is gone starts a new one.
        let (tx, rx) = channel();
        drop(rx);
        worker.tx = Some(tx);
        let out = temp("restart");
        let cmd = format!("echo restarted > '{}'", out.display());
        worker.send(Job { chord: "x".into(), down: true, action: Action::Shell(cmd) }).unwrap();
        assert_eq!(read_when_written(&out), "restarted\n");
        let _ = std::fs::remove_file(&out);
    }
}
