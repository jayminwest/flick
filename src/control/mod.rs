//! The control socket: other programs drive Flick with `["<module>","<verb>",args...]`
//! requests (protocol in `crate::core::control`) and subscribe to its events. Requests run
//! on the main thread, where every module lives; the socket threads only wait. `net` serves
//! the same protocol over TCP to allowed tailnet peers, when the remote module turns it on.

pub mod net;
pub mod server;
pub mod tailscale;

use std::path::PathBuf;
use std::sync::mpsc;

use crate::config;
use crate::core::Event;
use crate::core::control::{Flags, Reply, split_flags};
use crate::platform::events;
use server::Hub;

/// Event subscribers of this process.
static HUB: Hub = Hub::new();

/// `$FLICK_SOCKET`, else `flick.sock` in Flick's data directory.
pub fn socket_path() -> PathBuf {
    std::env::var_os("FLICK_SOCKET")
        .map_or_else(|| config::data_dir().join("flick.sock"), PathBuf::from)
}

/// Another Flick serves the control socket, whatever binary or bundle it runs from.
pub fn running() -> bool {
    server::answering(&socket_path())
}

/// Serve the control socket on background threads.
pub fn start() -> Result<(), String> {
    let path = socket_path();
    let listener = server::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    server::spawn(listener, on_main, &HUB).map_err(|e| e.to_string())
}

/// Stream `event` to subscribers. Costs nothing while there are none.
pub fn publish(event: Event) {
    HUB.publish(|| serde_json::to_string(&event).unwrap_or_default());
}

/// Run a request on the main thread and wait for its reply. Trailing `--json` and
/// `--remote` words are taken off and set `Cx::json` and `Cx::remote`.
fn on_main(words: Vec<String>) -> Reply {
    let (words, flags) = split_flags(words);
    run(words, flags)
}

/// Run request `words` with `flags` on the main thread and wait for its reply.
fn run(words: Vec<String>, flags: Flags) -> Reply {
    let (tx, rx) = mpsc::channel();
    events::on_main(move || {
        let _ = tx.send(crate::app::control(&words, flags));
    });
    rx.recv().map_or_else(
        |_| Reply::Error("the request failed inside Flick".into()),
        |result| Reply::answer(result, flags.json),
    )
}

/// Run request `words` as a local caller on the main queue, after whatever runs now, and
/// log an error reply. For in-process triggers that fire while the controller holds its
/// state (the `keys` module's `flick` chord actions): they cannot call `app::control`
/// directly, and nothing waits for the reply.
pub fn local(words: Vec<String>) {
    events::on_main(move || {
        if let Err(e) = crate::app::control(&words, Flags::default()) {
            eprintln!("flick: {}: {e}", words.join(" "));
        }
    });
}
