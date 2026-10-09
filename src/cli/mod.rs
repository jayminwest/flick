//! The command line: `flick` with no arguments runs the launcher; anything else is a
//! subcommand. `snapshot` and `import-raycast` run in this process; every other command is
//! a request to the running Flick over its control socket.

mod client;

use std::ffi::OsStr;

use crate::core::control::Flags;
use crate::core::store;
use crate::platform::app as macos;
use crate::{app, config, control, raycast, ui};

const USAGE: &str = "usage: flick                              run the launcher
       flick [--json] <module> <verb> [args]  ask the running Flick (--json: raw reply;
                                          FLICK_REMOTE set: marks the request --remote)
       flick reload                       reload config.toml
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
    /// No command after `--json`.
    Usage,
    Events,
    /// A control request, printed raw with `flags.json`; `flags.remote` sends `--remote`.
    Request {
        words: Vec<String>,
        flags: Flags,
    },
}

/// Parse the arguments after the program name. `--json` may come first or last.
/// `flick_remote` is the value of `$FLICK_REMOTE`: when set and not empty, a request is
/// marked `--remote` (an agent session that may send the output to a remote model).
pub fn parse(args: &[String], flick_remote: Option<&OsStr>) -> Command {
    let Some(first) = args.first() else { return Command::Launch };
    match first.as_str() {
        // Older macOS passes a process serial number to apps opened from Finder.
        psn if psn.starts_with("-psn_") => Command::Launch,
        "snapshot" => Command::Snapshot(args[1..].to_vec()),
        "import-raycast" => Command::ImportRaycast(args.get(1).cloned()),
        "help" | "--help" | "-h" => Command::Help,
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
                [w] if w == "events" => Command::Events,
                _ => {
                    let remote = flick_remote.is_some_and(|v| !v.is_empty());
                    Command::Request { words, flags: Flags { json, remote } }
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
        Command::Help => {
            println!("{}", usage());
            0
        }
        Command::Usage => {
            eprintln!("{}", usage());
            2
        }
        Command::Events => client::events(&control::socket_path()),
        Command::Request { words, flags } => {
            client::request(&control::socket_path(), &words, flags)
        }
    }
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

    fn parsed_with(args: &[&str], flick_remote: Option<&str>) -> Command {
        let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        parse(&args, flick_remote.map(OsStr::new))
    }

    fn parsed(args: &[&str]) -> Command {
        parsed_with(args, None)
    }

    fn request_with(words: &[&str], json: bool, remote: bool) -> Command {
        let words = words.iter().map(|s| (*s).to_string()).collect();
        Command::Request { words, flags: Flags { json, remote } }
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
    }

    #[test]
    fn everything_else_is_a_request() {
        assert_eq!(parsed(&["clip", "list"]), request(&["clip", "list"], false));
        assert_eq!(parsed(&["--json", "clip", "list"]), request(&["clip", "list"], true));
        assert_eq!(parsed(&["clip", "list", "--json"]), request(&["clip", "list"], true));
        assert_eq!(parsed(&["reload"]), request(&["reload"], false));
        assert_eq!(parsed(&["clip", "--json", "x"]), request(&["clip", "--json", "x"], false));
        assert_eq!(parsed(&["events"]), Command::Events);
        assert_eq!(parsed(&["--json", "events"]), Command::Events);
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
        assert_eq!(remote(&["events"], Some("1")), Command::Events);
        assert_eq!(remote(&["--help"], Some("1")), Command::Help);
        assert_eq!(remote(&[], Some("1")), Command::Launch);
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
