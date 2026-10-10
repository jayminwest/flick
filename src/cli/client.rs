//! The control client: send one request and print its reply, or print the event stream.
//! It talks to the local Flick over its Unix socket, or to another Mac's Flick over TCP
//! (`flick --host`, `$FLICK_HOST`).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::core::control::{
    DEFAULT_PORT, EVENTS, Flags, JSON, PeerHooks, REMOTE, Reply, request_line,
};

/// How long a TCP connect to each resolved address may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A Flick on another machine: a host name (a `MagicDNS` name or an IP address) and a port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub name: String,
    pub port: u16,
}

impl Host {
    /// Parse `name[:port]`; the port defaults to `DEFAULT_PORT`. An IPv6 address takes a
    /// port only in brackets (`[fd7a::1]:7419`); without them every colon is the address's.
    pub fn parse(spec: &str) -> Result<Host, String> {
        let bad = || format!("bad host {spec:?} (want name[:port])");
        let (name, port) = match spec.strip_prefix('[') {
            Some(rest) => {
                let (name, after) = rest.split_once(']').ok_or_else(bad)?;
                match after {
                    "" => (name, None),
                    _ => (name, Some(after.strip_prefix(':').ok_or_else(bad)?)),
                }
            }
            None => match spec.split_once(':') {
                Some((name, port)) if !port.contains(':') => (name, Some(port)),
                _ => (spec, None),
            },
        };
        let port = match port {
            Some(port) => port.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(bad)?,
            None => DEFAULT_PORT,
        };
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Err(bad());
        }
        Ok(Host { name: name.to_string(), port })
    }
}

/// Where a request goes: the local socket at a path, or a Flick on another machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Socket(PathBuf),
    Host(Host),
}

/// A connected control stream of either transport.
trait Conn: Read + Write {}
impl<T: Read + Write> Conn for T {}

/// The exit code of a request that got no reply: Flick not running or unreachable, the
/// connection dropped. Distinct from 1 (Flick answered with an error) so a script can tell
/// "fix the request" from "try another way" (KOTA falls back to iMessage only on this).
pub const UNREACHABLE: i32 = 3;

/// Send `words` to the Flick at `target` and print the reply: its text, or with `flags.json`
/// the raw reply line. With `flags.json` the request ends in `--json`, so a module may answer
/// with structured JSON; `flags.remote` adds `--remote` before it. Returns the exit code: 0
/// for an ok reply, 1 for an error reply, `UNREACHABLE` when no reply came.
pub fn request(target: &Target, words: &[String], flags: Flags) -> i32 {
    let json = flags.json;
    let words = with_flags(words, flags);
    let reply = connect(target).and_then(|stream| exchange(stream, &words));
    match reply {
        Ok(line) => {
            let (out, err, code) = render(&line, json);
            print!("{out}");
            eprint!("{err}");
            code
        }
        Err(e) => {
            eprintln!("flick: {e}");
            UNREACHABLE
        }
    }
}

/// How long a peer question (`PEER`) waits for its reply line once connected.
const PEER_REPLY: Duration = Duration::from_secs(10);

/// The client side of `core::control::PeerHooks`, passed to modules on their `modules!`
/// line so they can ask another Mac's Flick without importing this module.
pub const PEER: PeerHooks =
    PeerHooks { ask: |host, words, flags| ask(host, words, flags, PEER_REPLY) };

/// Send `words` (plus `--remote`, and `--json` with `flags.json`) to the Flick at `host`
/// (`name[:port]`) and read its reply, waiting at most `reply` for it.
fn ask(host: &str, words: &[String], flags: Flags, reply: Duration) -> Result<Reply, String> {
    let host = Host::parse(host)?;
    let unreachable =
        |e: io::Error| format!("can't reach Flick at {}:{} ({e})", host.name, host.port);
    let stream = connect_tcp(&host).map_err(unreachable)?;
    stream
        .set_read_timeout(Some(reply))
        .and_then(|()| stream.set_write_timeout(Some(reply)))
        .map_err(unreachable)?;
    let words = with_flags(words, Flags { remote: true, ..flags });
    let line = exchange(Box::new(stream), &words)
        .map_err(|e| format!("no reply from {}: {e}", host.name))?;
    Reply::parse(&line)
}

/// Subscribe to the Flick at `target` and print each event line until it goes away.
pub fn events(target: &Target) -> i32 {
    let streamed = connect(target).and_then(|stream| stream_events(stream, &mut io::stdout()));
    match streamed {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("flick: {e}");
            1
        }
    }
}

/// Ask `stream` for events and copy each line to `out`. An error reply instead of the
/// stream (a Flick that refuses events from the network) is an error.
fn stream_events(mut stream: Box<dyn Conn>, out: &mut dyn Write) -> io::Result<()> {
    writeln!(stream, "{}", request_line(&[EVENTS.to_string()]))?;
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if let Ok(Reply::Error(e)) = Reply::parse(&line) {
            return Err(io::Error::other(e));
        }
        writeln!(out, "{line}")?;
        out.flush()?;
    }
    Ok(())
}

fn connect(target: &Target) -> io::Result<Box<dyn Conn>> {
    match target {
        Target::Socket(path) => match UnixStream::connect(path) {
            Ok(stream) => Ok(Box::new(stream)),
            Err(e) => Err(io::Error::new(
                e.kind(),
                format!("can't reach Flick at {} ({e}); is it running?", path.display()),
            )),
        },
        Target::Host(host) => match connect_tcp(host) {
            Ok(stream) => Ok(Box::new(stream)),
            Err(e) => Err(io::Error::new(
                e.kind(),
                format!(
                    "can't reach Flick at {}:{} ({e}); is its network access on?",
                    host.name, host.port
                ),
            )),
        },
    }
}

/// Resolve `host` (`MagicDNS` names resolve like any other) and connect to the first address
/// that answers within `CONNECT_TIMEOUT`.
fn connect_tcp(host: &Host) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    for addr in (host.name.as_str(), host.port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                return Ok(stream);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// The request words: `words`, then `--remote` when `flags.remote`, then `--json` when
/// `flags.json` (the order `core::control::split_flags` takes them off).
fn with_flags(words: &[String], flags: Flags) -> Vec<String> {
    let mut words = words.to_vec();
    if flags.remote {
        words.push(REMOTE.to_string());
    }
    if flags.json {
        words.push(JSON.to_string());
    }
    words
}

/// Write the request line for `words` and read the one reply line.
fn exchange(mut stream: Box<dyn Conn>, words: &[String]) -> io::Result<String> {
    writeln!(stream, "{}", request_line(words))?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    if line.is_empty() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Flick closed the connection"));
    }
    Ok(line)
}

/// What to print for reply `line`: (stdout, stderr, exit code).
fn render(line: &str, json: bool) -> (String, String, i32) {
    let reply = Reply::parse(line);
    let code = i32::from(!matches!(reply, Ok(Reply::Ok(_))));
    if json {
        return (format!("{}\n", line.trim_end()), String::new(), code);
    }
    match reply {
        Ok(Reply::Ok(Value::String(out))) if out.is_empty() || out.ends_with('\n') => {
            (out, String::new(), 0)
        }
        Ok(Reply::Ok(Value::String(out))) => (format!("{out}\n"), String::new(), 0),
        Ok(Reply::Ok(value)) => (format!("{value}\n"), String::new(), 0),
        Ok(Reply::Error(e)) | Err(e) => (String::new(), format!("flick: {e}\n"), 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::server::{self, Hub};

    #[test]
    fn replies_print_as_text_or_raw_json() {
        let ok = |s: &str| (s.to_string(), String::new(), 0);
        assert_eq!(render("{\"ok\":\"a\\nb\"}\n", false), ok("a\nb\n"));
        assert_eq!(render("{\"ok\":\"a\\n\"}", false), ok("a\n"));
        assert_eq!(render("{\"ok\":\"\"}", false), ok(""));
        assert_eq!(render("{\"ok\":\"a\"}\n", true), ok("{\"ok\":\"a\"}\n"));
        assert_eq!(render("{\"error\":\"no\"}\n", false), (String::new(), "flick: no\n".into(), 1));
        assert_eq!(
            render("{\"error\":\"no\"}", true),
            ("{\"error\":\"no\"}\n".into(), String::new(), 1)
        );
        assert_eq!(render("{\"ok\":{\"a\":[1]}}", false), ok("{\"a\":[1]}\n"));
        assert_eq!(render("{\"ok\":[1]}\n", true), ok("{\"ok\":[1]}\n"));
        let (out, err, code) = render("garbage", false);
        assert!(out.is_empty() && err.starts_with("flick: bad reply") && code == 1);
    }

    fn upper(words: Vec<String>) -> Reply {
        let text = words.into_iter().map(|w| w.to_uppercase()).collect::<Vec<_>>().join(" ");
        Reply::Ok(text.into())
    }

    #[test]
    fn host_specs_default_the_port_and_bracket_ipv6() {
        let ok = |name: &str, port| Ok(Host { name: name.into(), port });
        assert_eq!(Host::parse("mbp-server"), ok("mbp-server", DEFAULT_PORT));
        assert_eq!(Host::parse("mbp-server:9000"), ok("mbp-server", 9000));
        assert_eq!(Host::parse("100.64.0.1:1"), ok("100.64.0.1", 1));
        assert_eq!(Host::parse("fd7a:115c:a1e0::1"), ok("fd7a:115c:a1e0::1", DEFAULT_PORT));
        assert_eq!(Host::parse("[fd7a::1]"), ok("fd7a::1", DEFAULT_PORT));
        assert_eq!(Host::parse("[fd7a::1]:80"), ok("fd7a::1", 80));
        for bad in ["", ":80", "a:", "a:x", "a:0", "a:70000", "[fd7a::1", "[::1]80", "[]", "a b"] {
            assert!(Host::parse(bad).is_err(), "{bad:?}");
        }
    }

    /// A fake Flick on 127.0.0.1: answers each request line with `reply(words)` or, for
    /// `["events"]`, sends `events` lines and hangs up. Returns its address as a `Host`.
    fn fake_tcp_flick(reply: fn(Vec<String>) -> Reply, events: &'static [&'static str]) -> Host {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let reader = BufReader::new(stream.try_clone().unwrap());
                for line in reader.lines() {
                    let words = crate::core::control::parse_request(&line.unwrap()).unwrap();
                    if words == [EVENTS] {
                        for event in events {
                            writeln!(stream, "{event}").unwrap();
                        }
                        break;
                    }
                    writeln!(stream, "{}", reply(words).to_line()).unwrap();
                }
            }
        });
        Host { name: "127.0.0.1".into(), port }
    }

    #[test]
    fn a_request_round_trips_over_tcp_with_its_flag_words() {
        let host = Target::Host(fake_tcp_flick(upper, &[]));
        let words = ["task".to_string(), "ls".into()];
        let tcp = connect(&host).unwrap();
        assert_eq!(
            exchange(tcp, &with_flags(&words, Flags { json: true, remote: true })).unwrap(),
            "{\"ok\":\"TASK LS --REMOTE --JSON\"}\n"
        );
        assert_eq!(request(&host, &words, Flags { json: false, remote: true }), 0);
        let refuse = |_| Reply::Error("refused".into());
        assert_eq!(
            request(&Target::Host(fake_tcp_flick(refuse, &[])), &words, Flags::default()),
            1
        );
    }

    #[test]
    fn events_stream_over_tcp_and_an_error_reply_fails() {
        let mut out = Vec::new();
        let host = Target::Host(fake_tcp_flick(upper, &["{\"a\":1}", "{\"b\":2}"]));
        stream_events(connect(&host).unwrap(), &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(events(&host), 0);
        let refused = Target::Host(fake_tcp_flick(upper, &["{\"error\":\"events are off\"}"]));
        let e = stream_events(connect(&refused).unwrap(), &mut Vec::new()).unwrap_err();
        assert_eq!(e.to_string(), "events are off");
        assert_eq!(events(&refused), 1);
    }

    #[test]
    fn an_unreachable_host_is_exit_3_with_its_address() {
        // Bind then drop: nothing listens on that port.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let host = Target::Host(Host { name: "127.0.0.1".into(), port });
        let e = connect(&host).err().unwrap().to_string();
        assert!(e.contains(&format!("127.0.0.1:{port}")) && e.contains("network access"), "{e}");
        assert_eq!(request(&host, &["x".into()], Flags::default()), UNREACHABLE);
        assert_eq!(events(&host), 1);
        let nowhere = Target::Host(Host { name: "no-such-host.invalid".into(), port });
        assert_eq!(request(&nowhere, &["x".into()], Flags::default()), UNREACHABLE);
    }

    #[test]
    fn a_peer_question_gets_the_reply_or_says_why_not() {
        let host = fake_tcp_flick(upper, &[]);
        let spec = format!("127.0.0.1:{}", host.port);
        let words = ["sys".to_string(), "snapshot".into()];
        let json = Flags { json: true, remote: false };
        // Always --remote, then --json; through the real `PEER` hook.
        assert_eq!(
            (PEER.ask)(&spec, &words, json),
            Ok(Reply::Ok("SYS SNAPSHOT --REMOTE --JSON".into()))
        );
        let refused = fake_tcp_flick(|_| Reply::Error("sys: unknown command".into()), &[]);
        let refused = format!("127.0.0.1:{}", refused.port);
        assert_eq!(
            ask(&refused, &words, json, PEER_REPLY),
            Ok(Reply::Error("sys: unknown command".into()))
        );
        assert!(ask("a b", &words, json, PEER_REPLY).unwrap_err().starts_with("bad host"));
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let e = ask(&format!("127.0.0.1:{port}"), &words, json, PEER_REPLY).unwrap_err();
        assert!(e.starts_with(&format!("can't reach Flick at 127.0.0.1:{port} (")), "{e}");
    }

    #[test]
    fn a_silent_peer_is_an_error_within_the_budget() {
        // Accepts, reads, never answers.
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let spec = format!("127.0.0.1:{}", silent.local_addr().unwrap().port());
        std::thread::spawn(move || {
            let held: Vec<_> = silent.incoming().take(1).collect();
            std::thread::sleep(Duration::from_secs(3));
            drop(held);
        });
        let start = std::time::Instant::now();
        let e =
            ask(&spec, &["x".into()], Flags::default(), Duration::from_millis(100)).unwrap_err();
        assert!(e.starts_with("no reply from 127.0.0.1: "), "{e}");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_request_round_trips_through_a_real_socket() {
        static HUB: Hub = Hub::new();
        let dir = std::env::temp_dir().join(format!("flk-{}-client", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.sock");
        let socket = Target::Socket(path.clone());
        let missing = connect(&socket).err().unwrap().to_string();
        assert!(missing.contains("is it running?"), "{missing}");
        assert_eq!(request(&socket, &["x".into()], Flags::default()), UNREACHABLE);
        server::spawn(server::bind(&path).unwrap(), upper, &HUB).unwrap();
        let words = ["clip".to_string(), "get".into(), "a b".into()];
        let line = exchange(connect(&socket).unwrap(), &words).unwrap();
        assert_eq!(line, "{\"ok\":\"CLIP GET A B\"}\n");
        assert_eq!(request(&socket, &words, Flags { json: true, remote: true }), 0);
    }

    #[test]
    fn flag_words_go_last_in_the_order_the_server_takes_them_off() {
        let words = ["task".to_string(), "ls".into()];
        let sent = |json, remote| with_flags(&words, Flags { json, remote });
        assert_eq!(sent(false, false), words);
        assert_eq!(sent(true, false), ["task", "ls", "--json"]);
        assert_eq!(sent(false, true), ["task", "ls", "--remote"]);
        let both = sent(true, true);
        assert_eq!(both, ["task", "ls", "--remote", "--json"]);
        assert_eq!(crate::core::control::split_flags(both).1, Flags { json: true, remote: true });
    }
}
