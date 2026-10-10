//! The real `io::Hooks`: curl through `std::process`, `ModuleChanged` through
//! `platform::events`. Tests use `testkit::HOOKS`, so no test runs curl.

use std::process::{Command, Stdio};

use super::ID;
use super::io::Hooks;
use super::transport::{Child, Spawned};
use crate::core::Event;
use crate::platform::events;

pub const HOOKS: Hooks = Hooks { spawn, post, sleep: std::thread::sleep };

fn post() {
    events::post(Event::ModuleChanged { module: ID });
}

impl Child for std::process::Child {
    fn kill(&mut self) {
        let _ = std::process::Child::kill(self);
    }

    fn wait(&mut self) -> Option<i32> {
        std::process::Child::wait(self).ok().and_then(|s| s.code())
    }
}

/// Start `argv` (always `/usr/bin/curl`, `-q` first) with all three pipes.
fn spawn(argv: &[String]) -> Result<Spawned, String> {
    let (program, rest) = argv.split_first().ok_or("empty command")?;
    let mut child = Command::new(program)
        .args(rest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = std::process::Child::kill(&mut child);
        return Err(format!("{program}: no pipes"));
    };
    Ok(Spawned { stdin: Box::new(stdin), stdout: Box::new(stdout), stderr: Box::new(stderr), child: Box::new(child) })
}
