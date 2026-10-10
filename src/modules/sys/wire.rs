//! The real `io::Hooks`: posting `ModuleChanged`, the clock, child processes, connects, the
//! uid, the launcher panel and the peer client the `modules!` line passes in. Tests use
//! `testkit::HOOKS`, so no test runs curl, launchctl, ssh or the probe, or asks a peer.

use super::io::Hooks;
use super::{ID, run, unix_now};
use crate::core::Event;
use crate::core::control::PeerHooks;
use crate::platform::{events, panel, workspace};

pub fn hooks(peer: PeerHooks) -> Hooks {
    Hooks {
        post: || events::post(Event::ModuleChanged { module: ID }),
        now: unix_now,
        run: run::run,
        connect: run::connect,
        uid: run::uid,
        run_input: run::run_input,
        ask: peer.ask,
        visible: panel::is_visible,
        open: workspace::open_url,
    }
}
