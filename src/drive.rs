//! The driven instance (`flick --drive`): the real launcher for an agent to drive and read
//! over a scratch control socket, with nothing on screen and nothing of the user's touched.
//!
//! It boots as `flick snapshot` does: the panel never shows, the store is in memory, and
//! there are no hotkeys, key tap, status items, platform observers or already-running check.
//! It reads its fixture config from `$FLICK_CONFIG` and serves the normal control socket on
//! `$FLICK_SOCKET`; it needs both, so it never reads the user's config (a `reload` included)
//! or takes the user's socket. `modules::DRIVEN_OFF` modules stay out.

use std::ffi::OsString;

use crate::core::store::Store;
use crate::platform::{app as macos, events};
use crate::{app, config, control, ui};

/// Run a driven instance until it quits. Returns the exit code when it cannot start.
pub fn launch() -> i32 {
    if let Err(e) = scratch(std::env::var_os("FLICK_CONFIG"), std::env::var_os("FLICK_SOCKET")) {
        eprintln!("flick: {e}");
        return 2;
    }
    macos::require_main_thread();
    let socket = control::socket_path();
    if control::running() {
        eprintln!("flick: a Flick already answers on {}", socket.display());
        return 1;
    }
    // Before any window exists.
    macos::set_driven();
    macos::set_accessory();
    let config = config::load().unwrap_or_else(|e| {
        eprintln!("flick: {e}; using defaults");
        config::Config::default()
    });
    ui::init();
    app::init(config, Store::in_memory());
    events::start_posted(app::on_event);
    if let Err(e) = control::start() {
        eprintln!("flick: control socket: {e}");
        return 1;
    }
    eprintln!("flick: driven on {}", socket.display());
    macos::run();
    0
}

/// `Err` unless both the config and the socket are set to paths of their own.
fn scratch(config: Option<OsString>, socket: Option<OsString>) -> Result<(), &'static str> {
    let set = |v: Option<OsString>| v.is_some_and(|v| !v.is_empty());
    if set(config) && set(socket) {
        Ok(())
    } else {
        Err("--drive needs FLICK_CONFIG and FLICK_SOCKET set to scratch paths")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_driven_instance_needs_a_scratch_config_and_socket() {
        let path = |s: &str| Some(OsString::from(s));
        assert!(scratch(path("/tmp/x/config.toml"), path("/tmp/x/flick.sock")).is_ok());
        for (config, socket) in [
            (None, path("/tmp/x/flick.sock")),
            (path("/tmp/x/config.toml"), None),
            (path(""), path("/tmp/x/flick.sock")),
            (path("/tmp/x/config.toml"), path("")),
            (None, None),
        ] {
            assert!(scratch(config.clone(), socket.clone()).is_err(), "{config:?} {socket:?}");
        }
    }
}
