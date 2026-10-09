//! The command line: `flick` with no arguments runs the launcher; anything else is a
//! subcommand. `snapshot`, `import-raycast` and `config example` run in this process; every other command is
//! a request to the running Flick over its control socket, or with `--host` (or
//! `$FLICK_HOST`) to another Mac's Flick over TCP.

mod client;

use std::ffi::OsStr;

use client::{Host, Target};

use crate::core::control::Flags;
use crate::core::store;
use crate::platform::app as macos;
use crate::{app, config, control, raycast, ui};

const USAGE: &str = "usage: flick                              run the launcher
       flick [--json] <module> <verb> [args]  ask the running Flick (--json: raw reply;
                                          FLICK_REMOTE set: marks the request --remote)
       flick --host <name[:port]> ...     ask the Flick on another Mac over Tailscale
                                          (or FLICK_HOST; always --remote)
       flick reload                       reload config.toml
       flick config example               print every config.toml option, commented
       flick events                       stream events as JSON lines
       flick snapshot <out.png> [query]
       flick import-raycast <Quicklinks.json>";

/// `USAGE` plus every module's verbs (`Module::verbs`), one module per line.
fn usage() -> String {
    let verbs = crate::modules::verbs().join("\n       ");
    format!("{USAGE}\n\nverbs: {verbs}")
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Run the launcher.
    Launch,
    /// Arguments after `snapshot`.
    Snapshot(Vec<String>),
    /// The file after `import-raycast`, if any.
    ImportRaycast(Option<String>),
    Help,
    /// Print `config.example.toml`.
    ConfigExample,
    /// No command after `--json`, or a bad or misplaced `--host`.
    Usage,
    /// Stream events from the local Flick, or from the one at the host.
    Events(Option<Host>),
    /// A control request, printed raw with `flags.json`; `flags.remote` sends `--remote`.
    /// With a host it goes over TCP to that Flick.
    Request {
        words: Vec<String>,
        flags: Flags,
        host: Option<Host>,
    },
}

/// Parse the arguments after the program name. `--json` may come first or last.
/// `flick_remote` is the value of `$FLICK_REMOTE`: when set and not empty, a request is
/// marked `--remote` (an agent session that may send the output to a remote model).
/// `--host <name[:port]>` first (or after a leading `--json`) sends requests and `events` to
/// that Flick; `flick_host` (`$FLICK_HOST`, when not empty) does the same when the flag is
/// absent. A host request is always `--remote`. `--host` with a command that runs in this
/// process is a usage error; `$FLICK_HOST` leaves such commands alone.
pub fn parse(args: &[String], flick_remote: Option<&OsStr>, flick_host: Option<&OsStr>) -> Command {
    let Ok((flag, args)) = take_host(args) else { return Command::Usage };
    let explicit = flag.is_some();
    let spec = flag
        .or_else(|| flick_host.filter(|v| !v.is_empty()).map(|v| v.to_string_lossy().into_owned()));
    let host = match spec.as_deref().map(Host::parse) {
        None => None,
        Some(Ok(host)) => Some(host),
        Some(Err(e)) => {
            eprintln!("flick: {e}");
            return Command::Usage;
        }
    };
    match (parse_local(&args, flick_remote), host) {
        (command, None) => command,
        (Command::Events(None), host) => Command::Events(host),
        (Command::Request { words, flags, .. }, host) => {
            Command::Request { words, flags: Flags { remote: true, ..flags }, host }
        }
        (_, Some(_)) if explicit => Command::Usage,
        (command, Some(_)) => command,
    }
}

/// Take a leading `--host <spec>` (or one right after a leading `--json`) off `args`.
/// Returns the spec, if any, and the other arguments; `Err` for `--host` without a spec.
fn take_host(args: &[String]) -> Result<(Option<String>, Vec<String>), ()> {
    let at = match args {
        [first, ..] if first == "--host" => 0,
        [first, second, ..] if first == "--json" && second == "--host" => 1,
        _ => return Ok((None, args.to_vec())),
    };
    let spec = args.get(at + 1).filter(|s| !s.starts_with('-')).ok_or(())?;
    let mut rest = args.to_vec();
    rest.drain(at..at + 2);
    Ok((Some(spec.clone()), rest))
}

/// `parse` without the host: the local meaning of `args`.
fn parse_local(args: &[String], flick_remote: Option<&OsStr>) -> Command {
    let Some(first) = args.first() else { return Command::Launch };
    match first.as_str() {
        // Older macOS passes a process serial number to apps opened from Finder.
        psn if psn.starts_with("-psn_") => Command::Launch,
        "snapshot" => Command::Snapshot(args[1..].to_vec()),
        "import-raycast" => Command::ImportRaycast(args.get(1).cloned()),
        "help" | "--help" | "-h" => Command::Help,
        "config" if args[1..] == ["example"] => Command::ConfigExample,
        _ => {
            let mut words = args.to_vec();
            let json = if first == "--json" {
                words.remove(0);
                true
            } else {
                words.pop_if(|w| w == "--json").is_some()
            };
            match words.as_slice() {
                [] => Command::Usage,
                [w] if w == "events" => Command::Events(None),
                _ => {
                    let remote = flick_remote.is_some_and(|v| !v.is_empty());
                    Command::Request { words, flags: Flags { json, remote }, host: None }
                }
            }
        }
    }
}

/// Run a command other than `Launch`. Returns the exit code.
pub fn run(command: Command) -> i32 {
    match command {
        Command::Launch => 0,
        Command::Snapshot(args) => snapshot(&args),
        Command::ImportRaycast(path) => import_raycast(path.as_deref()),
        Command::ConfigExample => {
            print!("{}", config::example::EXAMPLE);
            0
        }
        Command::Help => {
            println!("{}", usage());
            0
        }
        Command::Usage => {
            eprintln!("{}", usage());
            2
        }
        Command::Events(host) => client::events(&target(host)),
        Command::Request { words, flags, host } => client::request(&target(host), &words, flags),
    }
}

/// The local socket, or the Flick at `host`.
fn target(host: Option<Host>) -> Target {
    host.map_or_else(|| Target::Socket(control::socket_path()), Target::Host)
}

/// `flick snapshot <out.png> [query]`: draw the launcher to a PNG without showing it.
/// Uses the default config and an empty database, so no personal data appears.
fn snapshot(args: &[String]) -> i32 {
    let Some(out) = args.first() else {
        eprintln!("usage: flick snapshot <out.png> [query]");
        return 2;
    };
    macos::require_main_thread();
    ui::init();
    app::init(config::Config::default(), store::Store::in_memory());
    app::set_root_query(args.get(1).map_or("", String::as_str));
    if let Err(e) = ui::snapshot(out) {
        eprintln!("flick: {e}");
        return 1;
    }
    0
}

/// `flick import-raycast <Quicklinks.json>`: add Raycast's exported quicklinks to the config.
fn import_raycast(path: Option<&str>) -> i32 {
    let Some(path) = path else {
        eprintln!("usage: flick import-raycast <Quicklinks.json>");
        return 2;
    };
    match raycast::import_file(path.as_ref()) {
        Ok(report) => {
            println!("{report}");
            0
        }
        Err(e) => {
            eprintln!("flick: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_env(args: &[&str], flick_remote: Option<&str>, flick_host: Option<&str>) -> Command {
        let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        parse(&args, flick_remote.map(OsStr::new), flick_host.map(OsStr::new))
    }

    fn parsed_with(args: &[&str], flick_remote: Option<&str>) -> Command {
        parsed_env(args, flick_remote, None)
    }

    fn parsed(args: &[&str]) -> Command {
        parsed_with(args, None)
    }

    fn request_with(words: &[&str], json: bool, remote: bool) -> Command {
        let words = words.iter().map(|s| (*s).to_string()).collect();
        Command::Request { words, flags: Flags { json, remote }, host: None }
    }

    fn host(name: &str, port: u16) -> Host {
        Host { name: name.into(), port }
    }

    fn to_host(words: &[&str], json: bool, at: Host) -> Command {
        let words = words.iter().map(|s| (*s).to_string()).collect();
        Command::Request { words, flags: Flags { json, remote: true }, host: Some(at) }
    }

    fn request(words: &[&str], json: bool) -> Command {
        request_with(words, json, false)
    }

    #[test]
    fn subcommands_keep_their_arguments() {
        assert_eq!(parsed(&[]), Command::Launch);
        assert_eq!(parsed(&["-psn_0_123"]), Command::Launch);
        assert_eq!(parsed(&["snapshot"]), Command::Snapshot(vec![]));
        assert_eq!(
            parsed(&["snapshot", "a.png", "vsc", "extra"]),
            Command::Snapshot(vec!["a.png".into(), "vsc".into(), "extra".into()])
        );
        assert_eq!(parsed(&["import-raycast"]), Command::ImportRaycast(None));
        assert_eq!(
            parsed(&["import-raycast", "q.json", "x"]),
            Command::ImportRaycast(Some("q.json".into()))
        );
        assert_eq!(parsed(&["--help"]), Command::Help);
        assert_eq!(parsed(&["help"]), Command::Help);
        assert_eq!(parsed(&["config", "example"]), Command::ConfigExample);
        assert_eq!(parsed(&["config"]), request(&["config"], false));
    }

    #[test]
    fn everything_else_is_a_request() {
        assert_eq!(parsed(&["clip", "list"]), request(&["clip", "list"], false));
        assert_eq!(parsed(&["--json", "clip", "list"]), request(&["clip", "list"], true));
        assert_eq!(parsed(&["clip", "list", "--json"]), request(&["clip", "list"], true));
        assert_eq!(parsed(&["reload"]), request(&["reload"], false));
        assert_eq!(parsed(&["clip", "--json", "x"]), request(&["clip", "--json", "x"], false));
        assert_eq!(parsed(&["events"]), Command::Events(None));
        assert_eq!(parsed(&["--json", "events"]), Command::Events(None));
        assert_eq!(parsed(&["--json"]), Command::Usage);
    }

    #[test]
    fn a_non_empty_flick_remote_marks_requests_remote() {
        let remote = |args: &[&str], env| parsed_with(args, env);
        assert_eq!(remote(&["task", "ls"], Some("1")), request_with(&["task", "ls"], false, true));
        assert_eq!(
            remote(&["task", "ls", "--json"], Some("yes")),
            request_with(&["task", "ls"], true, true)
        );
        assert_eq!(remote(&["task", "ls"], Some("")), request(&["task", "ls"], false));
        assert_eq!(remote(&["task", "ls"], None), request(&["task", "ls"], false));
        // Only requests carry it: events, help and the launcher are unchanged.
        assert_eq!(remote(&["events"], Some("1")), Command::Events(None));
        assert_eq!(remote(&["--help"], Some("1")), Command::Help);
        assert_eq!(remote(&[], Some("1")), Command::Launch);
    }

    #[test]
    fn host_flag_sends_requests_and_events_to_that_flick_always_remote() {
        let mbp = host("mbp-server", 7419);
        assert_eq!(
            parsed(&["--host", "mbp-server", "task", "ls"]),
            to_host(&["task", "ls"], false, mbp.clone())
        );
        assert_eq!(
            parsed(&["--host", "mbp:9000", "--json", "task", "ls"]),
            to_host(&["task", "ls"], true, host("mbp", 9000))
        );
        assert_eq!(
            parsed(&["--json", "--host", "mbp-server", "task", "ls"]),
            to_host(&["task", "ls"], true, mbp.clone())
        );
        assert_eq!(
            parsed(&["--host", "mbp-server", "task", "ls", "--json"]),
            to_host(&["task", "ls"], true, mbp.clone())
        );
        assert_eq!(parsed(&["--host", "mbp-server", "events"]), Command::Events(Some(mbp)));
        // A missing, flag-like or bad spec, no command, or a command that runs here.
        for args in [
            &["--host"][..],
            &["--host", "--json", "task"],
            &["--host", "mbp:x", "task"],
            &["--host", "mbp:0", "task"],
            &["--host", "mbp"],
            &["--host", "mbp", "snapshot", "a.png"],
            &["--host", "mbp", "help"],
            &["--host", "mbp", "config", "example"],
        ] {
            assert_eq!(parsed(args), Command::Usage, "{args:?}");
        }
        // Only a leading --host counts; elsewhere it is a request word.
        assert_eq!(parsed(&["task", "--host", "x"]), request(&["task", "--host", "x"], false));
    }

    #[test]
    fn flick_host_sends_requests_there_but_the_flag_wins() {
        let env = |args: &[&str], h| parsed_env(args, None, h);
        let mbp = host("mbp-server", 7419);
        assert_eq!(
            env(&["task", "ls"], Some("mbp-server")),
            to_host(&["task", "ls"], false, mbp.clone())
        );
        assert_eq!(env(&["events"], Some("mbp-server")), Command::Events(Some(mbp)));
        assert_eq!(
            env(&["--host", "other:1", "task", "ls"], Some("mbp-server")),
            to_host(&["task", "ls"], false, host("other", 1))
        );
        assert_eq!(env(&["task", "ls"], Some("")), request(&["task", "ls"], false));
        assert_eq!(env(&["task"], Some("bad:port")), Command::Usage);
        // Commands that run in this process ignore it.
        assert_eq!(env(&[], Some("mbp-server")), Command::Launch);
        assert_eq!(env(&["help"], Some("mbp-server")), Command::Help);
        assert_eq!(env(&["snapshot"], Some("mbp-server")), Command::Snapshot(vec![]));
        assert_eq!(env(&["--json"], Some("mbp-server")), Command::Usage);
    }

    #[test]
    fn usage_lists_module_verbs_in_registration_order() {
        let text = usage();
        let (head, verbs) = text.split_once("\n\nverbs: ").unwrap();
        assert_eq!(head, USAGE);
        // Only the modules pinned here; a new module's verb line is its own.
        let known = ["app ", "window ", "clip "];
        let lines: Vec<&str> = verbs
            .lines()
            .map(str::trim)
            .filter(|l| known.iter().any(|k| l.starts_with(k)))
            .collect();
        assert_eq!(
            lines,
            [
                "app list | app open|quit|force-quit|reveal <name> | app running | app uninstall <name> --dry-run|--yes",
                "window list | window <action>, e.g. window left-half",
                "clip list | clip get <id>",
            ]
        );
    }
}
