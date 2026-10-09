//! Waiting on sockets with `poll(2)`. A thread blocked on a listener can then be woken
//! through a local pipe, not by a network connection to the listener: on macOS a connection
//! from this machine to its own Tailscale IPv6 address times out (flick-3d4c).

use std::ffi::{c_int, c_short, c_uint};
use std::io;
use std::os::fd::AsRawFd;
use std::time::Duration;

/// `struct pollfd` from macOS `<poll.h>`.
#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

const POLLIN: c_short = 0x0001;
const POLLERR: c_short = 0x0008;
const POLLHUP: c_short = 0x0010;
const POLLNVAL: c_short = 0x0020;

unsafe extern "C" {
    // `nfds_t` is `unsigned int` on macOS.
    fn poll(fds: *mut PollFd, nfds: c_uint, timeout: c_int) -> c_int;
}

/// Wait until one of `fds` is readable, hung up or in error, or `timeout` (`None`: no
/// limit) passes. For each of `fds` in order: whether it is ready. All `false` on a timeout.
pub fn readable(fds: &[&dyn AsRawFd], timeout: Option<Duration>) -> io::Result<Vec<bool>> {
    let mut polled: Vec<PollFd> =
        fds.iter().map(|fd| PollFd { fd: fd.as_raw_fd(), events: POLLIN, revents: 0 }).collect();
    let nfds = c_uint::try_from(polled.len()).map_err(io::Error::other)?;
    let timeout = timeout.map_or(-1, |t| c_int::try_from(t.as_millis()).unwrap_or(c_int::MAX));
    loop {
        // SAFETY: `polled` is a valid, writable array of `nfds` `pollfd`s for the call;
        // `poll` writes only their `revents`.
        let n = unsafe { poll(polled.as_mut_ptr(), nfds, timeout) };
        if n >= 0 {
            break;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
    let ready = POLLIN | POLLERR | POLLHUP | POLLNVAL;
    Ok(polled.iter().map(|p| p.revents & ready != 0).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    #[test]
    fn a_pipe_wakes_a_wait_on_a_listener_without_connecting_to_it() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let (wake, woken) = UnixStream::pair().unwrap();
        let fds: [&dyn AsRawFd; 2] = [&listener, &woken];
        let start = Instant::now();
        assert_eq!(readable(&fds, Some(Duration::from_millis(50))).unwrap(), [false, false]);
        assert!(start.elapsed() >= Duration::from_millis(40));

        let waiter = std::thread::spawn(move || readable(&[&listener, &woken], None).unwrap());
        std::thread::sleep(Duration::from_millis(20));
        wake.shutdown(Shutdown::Write).unwrap();
        assert_eq!(waiter.join().unwrap(), [false, true]);
    }

    #[test]
    fn a_pending_connection_or_byte_is_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let (mut wake, woken) = UnixStream::pair().unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let fds: [&dyn AsRawFd; 2] = [&listener, &woken];
        assert_eq!(readable(&fds, Some(Duration::from_secs(5))).unwrap(), [true, false]);
        wake.write_all(b"x").unwrap();
        assert_eq!(readable(&fds, Some(Duration::from_secs(5))).unwrap(), [true, true]);
        assert!(readable(&[], Some(Duration::ZERO)).unwrap().is_empty());
    }
}
