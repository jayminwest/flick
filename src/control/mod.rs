//! The control socket: other programs drive Flick with `["<module>","<verb>",args...]`
//! requests (protocol in `crate::core::control`) and subscribe to its events. Requests run
//! on the main thread, where every module lives; the socket threads only wait.

pub mod server;

use std::path::PathBuf;
use std::sync::mpsc;

use crate::config;
use crate::core::Event;
use crate::core::control::{Reply, split_json};
use crate::platform::events;
use server::Hub;

/// Event subscribers of this process.
static HUB: Hub = Hub::new();

/// `$FLICK_SOCKET`, else `flick.sock` in Flick's data directory.
pub fn socket_path() -> PathBuf {
    std::env::var_os("FLICK_SOCKET")
        .map_or_else(|| config::data_dir().join("flick.sock"), PathBuf::from)
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

/// Run a request on the main thread and wait for its reply. A trailing `--json` is taken
/// off the words and asks for a structured reply.
fn on_main(words: Vec<String>) -> Reply {
    let (words, json) = split_json(words);
    let (tx, rx) = mpsc::channel();
    events::on_main(move || {
        let _ = tx.send(crate::app::control(&words, json));
    });
    rx.recv().map_or_else(
        |_| Reply::Error("the request failed inside Flick".into()),
        |result| Reply::answer(result, json),
    )
}
