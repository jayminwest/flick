//! Runs a script command: `/bin/sh -c` on its own thread, so a slow command (ssh, a
//! network call) never holds the main thread. Output is dropped; a failure is logged and
//! posted as a notification with the last line of stderr.

use std::process::{Command, Stdio};
use std::thread;

use crate::platform::{events, notify};

pub fn run(name: &str, cmd: &str) {
    let (name, cmd) = (name.to_string(), cmd.to_string());
    let thread = thread::Builder::new().name("flick-script".into());
    let started = thread.spawn(move || {
        let out = Command::new("/bin/sh")
            .args(["-c", &cmd])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        let why = match out {
            Ok(o) if o.status.success() => return,
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let last = stderr.lines().rev().find(|l| !l.trim().is_empty());
                last.map_or_else(|| o.status.to_string(), |l| l.trim().to_string())
            }
            Err(e) => e.to_string(),
        };
        eprintln!("flick: script: {name}: {why}");
        events::on_main(move || {
            let _ = notify::post(&format!("script:{name}"), &format!("{name} failed"), &why);
        });
    });
    if let Err(e) = started {
        eprintln!("flick: script: cannot start a thread: {e}");
    }
}
