//! The socket server, plain std: a thread blocks in `accept`, and each connection gets a
//! thread that reads request lines and writes reply lines, or streams events. Nothing polls.

use std::fs::{self, Permissions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Mutex, PoisonError};
use std::thread;

use crate::core::control::{EVENTS, Reply, parse_request};

/// Answers one request's words. Runs on the connection's thread.
pub type Handler = fn(Vec<String>) -> Reply;

/// Event lines a subscriber may fall behind by before it is disconnected.
const BACKLOG: usize = 256;

/// Event subscribers: one channel per streaming connection.
pub struct Hub {
    subscribers: Mutex<Vec<SyncSender<String>>>,
}

impl Hub {
    pub const fn new() -> Hub {
        Hub { subscribers: Mutex::new(Vec::new()) }
    }

    /// Send `line()` to every subscriber; `line` runs only when there is one. Drops a
    /// subscriber that disconnected or fell `BACKLOG` lines behind. Never blocks on a client.
    pub fn publish(&self, line: impl FnOnce() -> String) {
        let mut subscribers = self.subscribers.lock().unwrap_or_else(PoisonError::into_inner);
        if subscribers.is_empty() {
            return;
        }
        let line = line();
        subscribers.retain(|tx| tx.try_send(line.clone()).is_ok());
    }

    fn subscribe(&self) -> Receiver<String> {
        let (tx, rx) = sync_channel(BACKLOG);
        self.subscribers.lock().unwrap_or_else(PoisonError::into_inner).push(tx);
        rx
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.subscribers.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

/// Listen at `path`, readable and writable by this user only (0600). A stale socket file
/// left by a crash is replaced; one another live process answers on is an error.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if UnixStream::connect(path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("{} is in use by another Flick", path.display()),
        ));
    }
    // Bind and chmod under a private name, then rename into place: the socket is never
    // reachable with looser permissions, and a stale file is replaced atomically.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}", std::process::id()));
    let tmp = Path::new(&tmp);
    let _ = fs::remove_file(tmp);
    let listener = UnixListener::bind(tmp)?;
    let placed = fs::set_permissions(tmp, Permissions::from_mode(0o600))
        .and_then(|()| fs::rename(tmp, path));
    if let Err(e) = placed {
        let _ = fs::remove_file(tmp);
        return Err(e);
    }
    Ok(listener)
}

/// Accept connections on `listener` on a background thread, one thread per connection.
pub fn spawn(listener: UnixListener, handler: Handler, hub: &'static Hub) -> io::Result<()> {
    thread::Builder::new().name("flick-control".into()).spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let _ = thread::Builder::new()
                        .name("flick-client".into())
                        .spawn(move || connection(stream, handler, hub));
                }
                Err(e) => eprintln!("flick: control socket: {e}"),
            }
        }
    })?;
    Ok(())
}

/// Serve one client: a reply line per request line until it hangs up, or, after an
/// `["events"]` request, event lines until it hangs up.
fn connection(stream: UnixStream, handler: Handler, hub: &Hub) -> io::Result<()> {
    let mut out = stream.try_clone()?;
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match parse_request(&line) {
            Ok(words) if words == [EVENTS] => return stream_events(&mut out, hub),
            Ok(words) => handler(words),
            Err(e) => Reply::Error(e),
        };
        writeln!(out, "{}", reply.to_line())?;
    }
    Ok(())
}

/// Write each published event line to `out`. Ends when the client hangs up (noticed at the
/// next write) or the hub drops this subscriber.
fn stream_events(out: &mut UnixStream, hub: &Hub) -> io::Result<()> {
    for line in hub.subscribe() {
        writeln!(out, "{line}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::FileTypeExt;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn echo(words: Vec<String>) -> Reply {
        match words.first().map(String::as_str) {
            Some("fail") => Reply::Error("failed".into()),
            _ => Reply::Ok(words.into_iter().reduce(|a, b| a + " " + &b).unwrap_or_default()),
        }
    }

    /// A socket path in a fresh directory. Short: macOS caps socket paths at 104 bytes.
    fn socket_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flk-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("f.sock")
    }

    fn start(path: &Path, hub: &'static Hub) {
        spawn(bind(path).unwrap(), echo, hub).unwrap();
    }

    fn ask(stream: &mut BufReader<UnixStream>, request: &str) -> String {
        writeln!(stream.get_mut(), "{request}").unwrap();
        let mut line = String::new();
        stream.read_line(&mut line).unwrap();
        line
    }

    fn connect(path: &Path) -> BufReader<UnixStream> {
        BufReader::new(UnixStream::connect(path).unwrap())
    }

    fn wait_for(what: &str, f: impl Fn() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn requests_get_one_reply_line_each() {
        static HUB: Hub = Hub::new();
        let path = socket_path("req");
        start(&path, &HUB);
        let mode = fs::metadata(&path).unwrap();
        assert!(mode.file_type().is_socket());
        assert_eq!(mode.permissions().mode() & 0o777, 0o600);

        let mut a = connect(&path);
        let mut b = connect(&path);
        assert_eq!(ask(&mut a, r#"["clip","list"]"#), "{\"ok\":\"clip list\"}\n");
        assert_eq!(ask(&mut b, r#"["fail"]"#), "{\"error\":\"failed\"}\n");
        writeln!(a.get_mut()).unwrap();
        assert_eq!(ask(&mut a, "not json"), ask(&mut b, "not json"));
        assert!(ask(&mut a, "[]").contains("empty request"));
        // A second server on a live socket refuses to steal it.
        assert_eq!(bind(&path).unwrap_err().kind(), io::ErrorKind::AddrInUse);
        assert_eq!(ask(&mut a, r#"["x"]"#), "{\"ok\":\"x\"}\n");
    }

    #[test]
    fn a_stale_socket_file_is_replaced() {
        static HUB: Hub = Hub::new();
        let path = socket_path("stale");
        drop(UnixListener::bind(&path).unwrap());
        assert!(UnixStream::connect(&path).is_err());
        start(&path, &HUB);
        assert_eq!(ask(&mut connect(&path), r#"["up"]"#), "{\"ok\":\"up\"}\n");
        let leftovers = fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn bind_fails_cleanly_in_a_missing_directory() {
        let path = socket_path("missing").join("nope").join("f.sock");
        assert_eq!(bind(&path).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn events_stream_to_every_subscriber_and_hung_up_ones_are_dropped() {
        static HUB: Hub = Hub::new();
        HUB.publish(|| unreachable!("no subscribers, so no line is built"));
        let path = socket_path("events");
        start(&path, &HUB);
        let mut a = connect(&path);
        let mut b = connect(&path);
        for s in [&mut a, &mut b] {
            writeln!(s.get_mut(), "[\"events\"]").unwrap();
        }
        wait_for("two subscribers", || HUB.len() == 2);
        HUB.publish(|| r#"{"event":"wake"}"#.into());
        for s in [&mut a, &mut b] {
            let mut line = String::new();
            s.read_line(&mut line).unwrap();
            assert_eq!(line, "{\"event\":\"wake\"}\n");
        }
        drop(a);
        // The hang-up shows at the next write; the one after finds the channel closed.
        wait_for("the hung-up subscriber to go", || {
            HUB.publish(|| r#"{"event":"active"}"#.into());
            HUB.len() == 1
        });
        let mut line = String::new();
        b.read_line(&mut line).unwrap();
        assert_eq!(line, "{\"event\":\"active\"}\n");
    }

    #[test]
    fn a_subscriber_that_falls_behind_is_dropped() {
        let hub = Hub::new();
        let rx = hub.subscribe();
        for _ in 0..=BACKLOG {
            hub.publish(|| "x".into());
        }
        assert_eq!(hub.len(), 0);
        assert_eq!(rx.iter().count(), BACKLOG);
    }
}
