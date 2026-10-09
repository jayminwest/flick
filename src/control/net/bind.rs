//! Binding the network listeners: each address once, a brief retry while the port is in
//! use, and an error that says which address failed and, from `lsof`, what holds it.
//! (flick-3d4c: `[fd7a:...]:7419: Address already in use` on a machine with one Flick.)

use std::io::{self, ErrorKind};
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::Path;
use std::thread;
use std::time::Duration;

use super::super::tailscale::run_as;

/// Binds to try while an address is in use, and the pause between them.
const TRIES: u32 = 5;
const PAUSE: Duration = Duration::from_millis(100);
/// How long `lsof` may take to name the holder of a port.
const LSOF_BUDGET: Duration = Duration::from_secs(2);
const LSOF: &str = "/usr/sbin/lsof";

/// `ips` without repeats, first order kept. `tailscale ip` may list an address twice, and a
/// second bind to it would fail as in use.
pub fn dedupe(ips: &[IpAddr]) -> Vec<IpAddr> {
    let mut seen = Vec::with_capacity(ips.len());
    for ip in ips {
        if !seen.contains(ip) {
            seen.push(*ip);
        }
    }
    seen
}

/// Bind `addr`. While it is in use, retry `TRIES` times `PAUSE` apart: a listener that
/// is closing (this process's, or a Flick that is quitting) may still hold it. On failure,
/// an error that names `addr` and, if it is in use, what holds it.
pub fn listen(addr: SocketAddr) -> Result<TcpListener, String> {
    listen_with(addr, TRIES, PAUSE)
        .map_err(|e| explain(addr, &e, || holder(Path::new(LSOF), addr, std::process::id())))
}

fn listen_with(addr: SocketAddr, tries: u32, pause: Duration) -> io::Result<TcpListener> {
    let mut attempt = 1;
    loop {
        // std sets SO_REUSEADDR (not SO_REUSEPORT): a connection of an old listener in
        // TIME_WAIT does not block the bind, but a live listener on `addr` does.
        match TcpListener::bind(addr) {
            Err(e) if e.kind() == ErrorKind::AddrInUse && attempt < tries => {
                attempt += 1;
                thread::sleep(pause);
            }
            result => return result,
        }
    }
}

/// The status error for a failed bind of `addr`. When it is in use, `holder` names what
/// holds it, if it can.
fn explain(addr: SocketAddr, e: &io::Error, holder: impl FnOnce() -> Option<String>) -> String {
    if e.kind() != ErrorKind::AddrInUse {
        return format!("{addr}: {e}");
    }
    let held = holder().unwrap_or_else(|| "another process (another Flick running?)".into());
    format!("{addr}: port {} is in use by {held}", addr.port())
}

/// What listens on `addr`, by `lsof` at `bin`: `None` if it cannot tell. `me` is this
/// process's id, so a listener of its own that did not close says so.
fn holder(bin: &Path, addr: SocketAddr, me: u32) -> Option<String> {
    let spec = format!("-iTCP@{addr}");
    let out = run_as("lsof", bin, &["-nP", &spec, "-sTCP:LISTEN", "-Fpc"], LSOF_BUDGET).ok()?;
    describe(&out, me)
}

/// The first process in `lsof -Fpc` output (`p<pid>` then `c<command>` lines).
fn describe(out: &str, me: u32) -> Option<String> {
    let pid: u32 = out.lines().find_map(|l| l.strip_prefix('p'))?.parse().ok()?;
    let command = out.lines().find_map(|l| l.strip_prefix('c')).unwrap_or("unknown");
    Some(if pid == me {
        format!("this Flick (pid {pid}): an earlier listener did not close")
    } else if command.to_ascii_lowercase().contains("flick") {
        format!("{command} (pid {pid}): another Flick is running")
    } else {
        format!("{command} (pid {pid})")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn repeated_addresses_are_bound_once() {
        let ips: Vec<IpAddr> =
            ["100.64.0.1", "fd7a:115c:a1e0::1", "100.64.0.1", "fd7a:115c:a1e0:0::1"]
                .iter()
                .map(|ip| ip.parse().unwrap())
                .collect();
        assert_eq!(dedupe(&ips), ips[..2]);
        assert!(dedupe(&[]).is_empty());
    }

    #[test]
    fn a_port_freed_while_retrying_is_bound() {
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = held.local_addr().unwrap();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            drop(held);
        });
        let start = Instant::now();
        let listener = listen(addr).unwrap();
        assert_eq!(listener.local_addr().unwrap(), addr);
        assert!(start.elapsed() >= Duration::from_millis(100));
        release.join().unwrap();
    }

    #[test]
    fn a_port_held_by_this_process_names_it() {
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = held.local_addr().unwrap();
        let e = listen_with(addr, 2, Duration::from_millis(1)).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::AddrInUse);
        let text = listen(addr).unwrap_err();
        assert!(
            text.starts_with(&format!(
                "127.0.0.1:{}: port {} is in use by ",
                addr.port(),
                addr.port()
            )),
            "{text}"
        );
        // `lsof` names this test process when it can; else the generic hint.
        assert!(
            text.contains(&format!("this Flick (pid {})", std::process::id()))
                || text.ends_with("another process (another Flick running?)"),
            "{text}"
        );
    }

    #[test]
    fn the_error_says_which_address_failed_and_what_holds_it() {
        let addr: SocketAddr = "[fd7a:115c:a1e0::1]:7419".parse().unwrap();
        let in_use = io::Error::from(ErrorKind::AddrInUse);
        assert_eq!(
            explain(addr, &in_use, || None),
            "[fd7a:115c:a1e0::1]:7419: port 7419 is in use by another process (another Flick running?)"
        );
        assert_eq!(
            explain(addr, &in_use, || Some("Flick (pid 7)".into())),
            "[fd7a:115c:a1e0::1]:7419: port 7419 is in use by Flick (pid 7)"
        );
        let other = io::Error::from(ErrorKind::AddrNotAvailable);
        let text = explain(addr, &other, || Some("never asked".into()));
        assert!(text.starts_with("[fd7a:115c:a1e0::1]:7419: "), "{text}");
        assert!(!text.contains("never asked"), "{text}");
    }

    #[test]
    fn lsof_output_names_the_holder() {
        assert_eq!(
            describe("p37283\ncFlick\nf8\n", 1).as_deref(),
            Some("Flick (pid 37283): another Flick is running")
        );
        assert_eq!(
            describe("p42\ncflick\nf5\n", 42).as_deref(),
            Some("this Flick (pid 42): an earlier listener did not close")
        );
        assert_eq!(describe("p9\ncnc\n", 1).as_deref(), Some("nc (pid 9)"));
        assert_eq!(describe("p9\n", 1).as_deref(), Some("unknown (pid 9)"));
        assert_eq!(describe("", 1), None);
        assert_eq!(describe("pnope\n", 1), None);
    }

    #[test]
    fn a_missing_lsof_or_a_free_port_names_nothing() {
        let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert_eq!(holder(Path::new("/nonexistent/lsof"), addr, 1), None);
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert_eq!(holder(Path::new(LSOF), free, 1), None);
    }
}
