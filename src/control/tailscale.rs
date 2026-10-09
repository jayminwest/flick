//! The seam between the network transport and Tailscale: this machine's tailnet addresses
//! and the names of a connecting peer. Production asks the `tailscale` CLI under a time
//! budget (`Cli`) and caches whois answers by address (`Cached`); tests pass fakes and never
//! run the CLI.

use std::collections::HashMap;
use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

/// What the transport needs from Tailscale.
pub trait Tailnet: Send + Sync {
    /// This machine's Tailscale addresses.
    fn ips(&self) -> Result<Vec<IpAddr>, String>;
    /// The names Tailscale knows the peer at `peer` by. `Err`, or no names, refuses it.
    fn whois(&self, peer: SocketAddr) -> Result<Vec<String>, String>;
}

/// Where the CLI may be: Homebrew (Apple silicon, Intel), then the macOS app's binary.
const CANDIDATES: &[&str] = &[
    "/opt/homebrew/bin/tailscale",
    "/usr/local/bin/tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];

/// How long one CLI call may take before it is killed (tailscaled down can hang it).
const BUDGET: Duration = Duration::from_secs(2);

/// How long a whois answer is reused: names for long, errors briefly.
const KEEP_NAMES: Duration = Duration::from_secs(60);
const KEEP_ERROR: Duration = Duration::from_secs(5);

/// The `tailscale` CLI.
pub struct Cli;

impl Tailnet for Cli {
    fn ips(&self) -> Result<Vec<IpAddr>, String> {
        parse_ips(&run(&find(CANDIDATES)?, &["ip"], BUDGET)?)
    }

    fn whois(&self, peer: SocketAddr) -> Result<Vec<String>, String> {
        let peer = peer.to_string();
        parse_whois(&run(&find(CANDIDATES)?, &["whois", "--json", &peer], BUDGET)?)
    }
}

/// The first of `candidates` that exists.
fn find(candidates: &[&str]) -> Result<PathBuf, String> {
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .ok_or_else(|| "tailscale CLI not found (is Tailscale installed?)".into())
}

/// Run `bin args` and return its stdout. Killed after `budget`; a failed exit is an error
/// with its stderr.
fn run(bin: &Path, args: &[&str], budget: Duration) -> Result<String, String> {
    run_as(&format!("tailscale {}", args.first().copied().unwrap_or_default()), bin, args, budget)
}

/// `run`, with errors prefixed by `name`.
pub(super) fn run_as(
    name: &str,
    bin: &Path,
    args: &[&str],
    budget: Duration,
) -> Result<String, String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{name}: {e}"))?;
    let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut o, mut e) = (String::new(), String::new());
        if let Some(out) = out.as_mut() {
            let _ = out.read_to_string(&mut o);
        }
        if let Some(err) = err.as_mut() {
            let _ = err.read_to_string(&mut e);
        }
        let _ = tx.send((o, e));
    });
    let Ok((out, err)) = rx.recv_timeout(budget) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("{name}: timed out after {}s", budget.as_secs()));
    };
    let status = child.wait().map_err(|e| format!("{name}: {e}"))?;
    if status.success() { Ok(out) } else { Err(format!("{name}: {}", err.trim())) }
}

/// The addresses in `tailscale ip` output, one per line.
fn parse_ips(out: &str) -> Result<Vec<IpAddr>, String> {
    let ips: Vec<IpAddr> = out.lines().filter_map(|l| l.trim().parse().ok()).collect();
    if ips.is_empty() {
        return Err("tailscale ip: no addresses (is Tailscale up?)".into());
    }
    Ok(ips)
}

/// The peer's names in `tailscale whois --json` output: `Node.ComputedName`,
/// `Node.Hostinfo.Hostname` and `Node.Name`, those present and not empty.
fn parse_whois(out: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(out).map_err(|e| format!("tailscale whois: {e}"))?;
    let node = &value["Node"];
    let names: Vec<String> = [&node["ComputedName"], &node["Hostinfo"]["Hostname"], &node["Name"]]
        .into_iter()
        .filter_map(Value::as_str)
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect();
    if names.is_empty() {
        return Err("tailscale whois: no node name".into());
    }
    Ok(names)
}

/// A whois answer and when it stops being reused.
type Entry = (Instant, Result<Vec<String>, String>);

/// `T` with whois answers cached by peer address (not port).
pub struct Cached<T> {
    inner: T,
    cache: Mutex<HashMap<IpAddr, Entry>>,
}

impl<T: Tailnet> Cached<T> {
    pub fn new(inner: T) -> Cached<T> {
        Cached { inner, cache: Mutex::new(HashMap::new()) }
    }

    fn whois_at(&self, peer: SocketAddr, now: Instant) -> Result<Vec<String>, String> {
        let hit = {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            cache.retain(|_, (until, _)| *until > now);
            cache.get(&peer.ip()).map(|(_, answer)| answer.clone())
        };
        if let Some(answer) = hit {
            return answer;
        }
        // The lock is not held across the CLI call: another peer need not wait for it.
        let answer = self.inner.whois(peer);
        let keep = if answer.is_ok() { KEEP_NAMES } else { KEEP_ERROR };
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.insert(peer.ip(), (now + keep, answer.clone()));
        answer
    }
}

impl<T: Tailnet> Tailnet for Cached<T> {
    fn ips(&self) -> Result<Vec<IpAddr>, String> {
        self.inner.ips()
    }

    fn whois(&self, peer: SocketAddr) -> Result<Vec<String>, String> {
        self.whois_at(peer, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn ip_output_parses_and_empty_output_is_an_error() {
        let ips = parse_ips("100.101.102.103\nfd7a:115c:a1e0::1\n\nnoise\n").unwrap();
        assert_eq!(
            ips,
            ["100.101.102.103".parse::<IpAddr>().unwrap(), "fd7a:115c:a1e0::1".parse().unwrap()]
        );
        assert!(parse_ips("").unwrap_err().contains("no addresses"));
    }

    #[test]
    fn whois_output_gives_every_node_name() {
        let out = r#"{"Node":{"Name":"mbp-server.tail1.ts.net.","ComputedName":"mbp-server",
            "Hostinfo":{"Hostname":"MBP-Server"}},"UserProfile":{"LoginName":"x@y"}}"#;
        assert_eq!(
            parse_whois(out).unwrap(),
            ["mbp-server", "MBP-Server", "mbp-server.tail1.ts.net."]
        );
        let partial = r#"{"Node":{"Name":"","ComputedName":"a"}}"#;
        assert_eq!(parse_whois(partial).unwrap(), ["a"]);
        assert!(parse_whois(r#"{"Node":{}}"#).unwrap_err().contains("no node name"));
        assert!(parse_whois("{}").is_err());
        assert!(parse_whois("not json").unwrap_err().starts_with("tailscale whois:"));
    }

    #[test]
    fn find_takes_the_first_existing_file() {
        assert_eq!(
            find(&["/nonexistent/tailscale", "/bin/sh", "/bin/ls"]).unwrap(),
            PathBuf::from("/bin/sh")
        );
        assert!(find(&["/nonexistent/tailscale", "/bin"]).unwrap_err().contains("not found"));
    }

    #[test]
    fn run_returns_stdout_or_stderr_and_kills_at_the_budget() {
        let sh = Path::new("/bin/sh");
        let budget = Duration::from_secs(5);
        assert_eq!(run(sh, &["-c", "echo hi"], budget).unwrap(), "hi\n");
        assert_eq!(
            run(sh, &["-c", "echo no >&2; exit 1"], budget).unwrap_err(),
            "tailscale -c: no"
        );
        let start = Instant::now();
        let slow = run(sh, &["-c", "exec sleep 10"], Duration::from_millis(100)).unwrap_err();
        assert!(slow.contains("timed out"), "{slow}");
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(
            run(Path::new("/nonexistent/tailscale"), &["ip"], budget)
                .unwrap_err()
                .starts_with("tailscale ip:")
        );
    }

    /// Counts whois calls; peers on port 1 fail.
    struct Counting(AtomicUsize);

    impl Tailnet for Counting {
        fn ips(&self) -> Result<Vec<IpAddr>, String> {
            Ok(vec!["100.64.0.1".parse().unwrap()])
        }
        fn whois(&self, peer: SocketAddr) -> Result<Vec<String>, String> {
            self.0.fetch_add(1, Ordering::Relaxed);
            if peer.port() == 1 {
                Err("down".into())
            } else {
                Ok(vec![format!("n{}", peer.port())])
            }
        }
    }

    #[test]
    fn whois_answers_are_cached_by_address_for_a_while() {
        let tailnet = Cached::new(Counting(AtomicUsize::new(0)));
        let calls = || tailnet.inner.0.load(Ordering::Relaxed);
        let a: SocketAddr = "100.64.0.2:7".parse().unwrap();
        let other_port: SocketAddr = "100.64.0.2:8".parse().unwrap();
        let t0 = Instant::now();
        assert_eq!(tailnet.whois_at(a, t0).unwrap(), ["n7"]);
        assert_eq!(tailnet.whois_at(other_port, t0 + Duration::from_secs(59)).unwrap(), ["n7"]);
        assert_eq!(calls(), 1);
        assert_eq!(tailnet.whois_at(other_port, t0 + KEEP_NAMES).unwrap(), ["n8"]);
        assert_eq!(calls(), 2);

        let down: SocketAddr = "100.64.0.3:1".parse().unwrap();
        assert!(tailnet.whois_at(down, t0).is_err());
        assert!(tailnet.whois_at(down, t0 + Duration::from_secs(4)).is_err());
        assert_eq!(calls(), 3);
        assert!(tailnet.whois_at(down, t0 + KEEP_ERROR).is_err());
        assert_eq!(calls(), 4);
        assert_eq!(tailnet.ips().unwrap().len(), 1);
        assert!(tailnet.whois(a).is_ok());
    }
}
