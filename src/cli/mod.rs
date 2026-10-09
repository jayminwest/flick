//! The command line: `flick` with no arguments runs the launcher; anything else is a
//! subcommand. `snapshot` and `import-raycast` run in this process; every other command is
//! a request to the running Flick over its control socket.

mod client;

use crate::core::store;
use crate::platform::app as macos;
use crate::{app, config, control, raycast, ui};

const USAGE: &str = "usage: flick                              run the launcher
       flick [--json] <module> <verb> [args]  ask the running Flick (--json: raw reply)
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
    /// A control request, printed raw with `json`.
    Request {
        words: Vec<String>,
        json: bool,
    },
}

/// Parse the arguments after the program name. `--json` may come first or last.
pub fn parse(args: &[String]) -> Command {
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
                _ => Command::Request { words, json },
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
        Command::Request { words, json } => client::request(&control::socket_path(), &words, json),
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

    fn parsed(args: &[&str]) -> Command {
        parse(&args.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
    }

    fn request(words: &[&str], json: bool) -> Command {
        Command::Request { words: words.iter().map(|s| (*s).to_string()).collect(), json }
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
