//! Fakes for the module's tests. `run` answers from fixtures by argv, so no test runs the
//! probe, curl, launchctl, pgrep or a configured command; `connect` never opens a socket;
//! `run_input` never runs ssh, `ask` never reaches a peer and `open` only records the URL.
//! `WINDOW` opens no window: it records what the module drew in `SHOWN`.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::time::Duration;

use serde_json::json;

use super::io::Hooks;
use super::probe::PROBE;
use super::run::Exit;
use super::window::{self, Note};
use crate::platform::surface::{Row, Status};
use crate::core::control::{Flags, Reply};

/// The real probe output from mbp-server.
pub const SERVER: &str = include_str!("fixtures/probe_mbp_server.txt");
/// The real probe output from a desktop Mac without a battery.
pub const DESKTOP: &str = include_str!("fixtures/probe_desktop.txt");

pub const HOOKS: Hooks = Hooks {
    post: || {},
    now: || 1_000,
    run,
    connect,
    uid: || Some(501),
    run_input,
    ask,
    visible: || false,
    open,
};

thread_local! {
    /// The URLs `open` was asked for on this thread.
    pub static OPENED: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}

fn open(url: &str) {
    OPENED.with(|o| o.borrow_mut().push(url.to_string()));
}

/// Canned ssh runs by target (the argv's sixth word), the script on stdin answered as the
/// Mac would:
/// - `pro`: `DESKTOP`, uid 502, then each `@@ svc` the script asks for: launchd label `up`
///   running, anything else not loaded; pgrep `ollama` runs, anything else does not;
/// - `refused`: ssh's own failure, exit 255; `mute`: exit 0, no output; `slow`: 300 ms,
///   then like `pro`; anything else times out.
/// - an action's script (`exec ...`) on `pro`: a kickstart of `'up'` exits 0, any other 113;
///   a tail prints `tail via <target>: <script>`.
fn run_input(argv: &[String], script: &str, _budget: Duration) -> Result<Exit, String> {
    let target = argv.get(5).map_or("", String::as_str);
    if target == "pro" && script.starts_with("exec /bin/launchctl kickstart") {
        let up = script.ends_with("/\"'up'\n");
        return if up { exit(0, "", "") } else { exit(113, "", "Could not find service in domain for user gui: 502\n") };
    }
    if target == "pro" && script.starts_with("exec /usr/bin/tail") {
        return exit(0, &format!("tail via {target}: {script}"), "");
    }
    let answer = |target| match target {
        "pro" | "slow" => {
            let mut out = format!("{DESKTOP}@@ uid\n502\n");
            for line in script.lines().filter(|l| l.starts_with("out=$(")) {
                let i = line.split("@@ svc ").nth(1).and_then(|r| r.split('\\').next()).unwrap_or("?");
                let (text, rc) = if line.contains("launchctl") {
                    if line.contains("/\"'up' 2>&1") {
                        (include_str!("fixtures/launchctl_running.txt").trim_end().to_string(), 0)
                    } else {
                        (include_str!("fixtures/launchctl_missing.txt").trim_end().to_string(), 113)
                    }
                } else if line.contains("-x 'ollama'") {
                    ("812".to_string(), 0)
                } else {
                    (String::new(), 1)
                };
                let _ = write!(out, "@@ svc {i}\n{text}\n@@ rc {i} {rc}\n");
            }
            Ok(Exit { code: Some(0), stdout: out, stderr: String::new() })
        }
        "refused" => Ok(Exit {
            code: Some(255),
            stdout: String::new(),
            stderr: "ssh: connect to host pro port 22: Connection refused\n".into(),
        }),
        "mute" => Ok(Exit { code: Some(0), ..Exit::default() }),
        _ => Err("timed out after 10 s".into()),
    };
    if target == "slow" {
        std::thread::sleep(Duration::from_millis(300));
    }
    answer(target)
}

/// Canned peers by host: `server` answers a snapshot (`--json` and `--remote` required);
/// `old` is a Flick without the sys module; `odd` answers text; `busy` has no snapshot yet;
/// anything else is unreachable.
fn ask(host: &str, words: &[String], flags: Flags) -> Result<Reply, String> {
    assert_eq!(words, ["sys", "snapshot"]);
    assert!(flags.json && flags.remote);
    let snapshot = || Reply::Ok(json!({ "host": host, "cpu_load": [0.5, 0.4, 0.3], "services": [] }));
    match host {
        "server" => Ok(snapshot()),
        "old" => Ok(Reply::Error("unknown module \"sys\" (modules: app, clip)".into())),
        "odd" => Ok(Reply::Ok("text".into())),
        "busy" => Ok(Reply::Error("sys: no snapshot yet (probe still running)".into())),
        _ => Err(format!("can't reach Flick at {host}:7419 (Connection refused)")),
    }
}

#[expect(clippy::unnecessary_wraps, reason = "fake hooks answer as the real ones do")]
fn exit(code: i32, stdout: &str, stderr: &str) -> Result<Exit, String> {
    Ok(Exit { code: Some(code), stdout: stdout.into(), stderr: stderr.into() })
}

/// Canned answers by argv:
/// - the probe: `SERVER`;
/// - curl: `http://ok/` 200 in 12 ms, `http://slow/` 200 in 1.5 s, `http://gone/` 404,
///   anything else refused;
/// - launchctl print: `gui/501/up` running, `gui/501/killed` running after a kill, else not
///   loaded;
/// - pgrep -x: `syncthing` runs;
/// - `/fake/print <text>` prints its text; `/fake/sleep <ms>` sleeps, then prints 1;
/// - launchctl kickstart -k: `gui/501/up` exits 0, else 113 (the print rule below);
/// - tail -n 100 -- <path>: `last of <path>`, or no such file for a path ending `missing.log`.
fn run(argv: &[String], _budget: Duration) -> Result<Exit, String> {
    let words: Vec<&str> = argv.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["/bin/sh", "-c", script] if *script == PROBE => exit(0, SERVER, ""),
        ["/usr/bin/curl", .., "http://ok/"] => exit(0, "200 0.012", ""),
        ["/usr/bin/curl", .., "http://slow/"] => exit(0, "200 1.5", ""),
        ["/usr/bin/curl", .., "http://gone/"] => exit(0, "404 0.002", ""),
        ["/usr/bin/curl", ..] => exit(7, "000", "curl: (7) Failed to connect\n"),
        ["/bin/launchctl", "print", "gui/501/up"] => {
            exit(0, include_str!("fixtures/launchctl_running.txt"), "")
        }
        ["/bin/launchctl", "kickstart", "-k", "gui/501/up"] => exit(0, "", ""),
        ["/usr/bin/tail", "-n", "100", "--", path] if path.ends_with("missing.log") => {
            exit(1, "", &format!("tail: {path}: No such file or directory\n"))
        }
        ["/usr/bin/tail", "-n", "100", "--", path] => exit(0, &format!("last of {path}\n"), ""),
        ["/bin/launchctl", "print", "gui/501/killed"] => {
            exit(0, include_str!("fixtures/launchctl_signal.txt"), "")
        }
        ["/bin/launchctl", ..] => exit(113, "", include_str!("fixtures/launchctl_missing.txt")),
        ["/usr/bin/pgrep", "-x", "syncthing"] => exit(0, "917\n958\n", ""),
        ["/usr/bin/pgrep", ..] => exit(1, "", ""),
        ["/fake/print", text] => exit(0, text, ""),
        ["/fake/sleep", ms] => {
            std::thread::sleep(Duration::from_millis(ms.parse().unwrap_or(0)));
            exit(0, "1", "")
        }
        _ => Err(format!("{}: No such file or directory", words.first().unwrap_or(&""))),
    }
}

/// `open:<n>` connects in n ms; anything else is refused.
fn connect(target: &str, _budget: Duration) -> Result<Duration, String> {
    match target.strip_prefix("open:").and_then(|ms| ms.parse().ok()) {
        Some(ms) => Ok(Duration::from_millis(ms)),
        None => Err(format!("{target}: Connection refused")),
    }
}

/// What the fake fleet window holds, on this thread.
#[derive(Default)]
pub struct Shown {
    pub opened: u32,
    pub visible: bool,
    /// It has the keyboard (`show` gives it; tests take it away).
    pub key: bool,
    pub header: Option<(String, String, Status)>,
    /// Each bubble as `key|side|header|time|state|md`, each card as
    /// `key|card|title|labels|confirm <action>`.
    pub rows: Vec<String>,
    pub notice: Option<String>,
    /// Notes for the next `take`, as the handlers would queue them.
    pub notes: Vec<Note>,
    pub snapshots: Vec<String>,
}

thread_local! {
    pub static SHOWN: RefCell<Shown> = RefCell::default();
}

fn shown<R>(f: impl FnOnce(&mut Shown) -> R) -> R {
    SHOWN.with(|s| f(&mut s.borrow_mut()))
}

fn row(r: &Row) -> String {
    match *r {
        Row::Bubble { key, side, header, time, state, md, .. } => format!("{key}|{side:?}|{header}|{time}|{state:?}|{md}"),
        Row::Card { key, card, ui, .. } => {
            let off = |a: &crate::core::card::Action| if a.enabled() { "" } else { " (off)" };
            let labels: Vec<String> = card.actions.iter().map(|a| format!("{}{}", a.label, off(a))).collect();
            format!("{key}|card|{}|{}|confirm {}", card.title, labels.join(", "), ui.confirm.unwrap_or("-"))
        }
        Row::Divider { .. } => "other".into(),
    }
}

/// A fake "fleet" surface; `snapshot` fails for a path under `/nope/`.
pub const WINDOW: window::Hooks = window::Hooks {
    open: || shown(|s| s.opened += 1),
    show: || shown(|s| (s.visible, s.key) = (true, true)),
    hide: || shown(|s| (s.visible, s.key) = (false, false)),
    visible: || shown(|s| s.visible),
    key: || shown(|s| s.key),
    header: |t, sub, st| shown(|s| s.header = Some((t.into(), sub.into(), st))),
    rows: |rows| shown(|s| s.rows = rows.iter().map(row).collect()),
    notice: |n| shown(|s| s.notice = n.map(str::to_string)),
    take: || shown(|s| std::mem::take(&mut s.notes)),
    snapshot: |path| {
        if path.starts_with("/nope/") {
            return Err(format!("can't write {path}"));
        }
        shown(|s| s.snapshots.push(path.into()));
        Ok(())
    },
};
