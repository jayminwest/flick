//! A fake `herdr` shell script that answers like the 0.9.1 CLI. Replies are synthesized
//! from the schema; none hold real terminal output.

use super::*;
use std::os::unix::fs::PermissionsExt;

/// A temp dir holding an executable `herdr` with `body` as its script. Each call appends
/// its arguments to `args.log` beside it.
struct Fake {
    dir: PathBuf,
    remote: Remote,
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fake {
    fn args(&self) -> Vec<String> {
        let log = std::fs::read_to_string(self.dir.join("args.log")).unwrap_or_default();
        log.lines().map(str::to_string).collect()
    }
}

fn fake(name: &str, body: &str) -> Fake {
    let dir = std::env::temp_dir().join(format!("flk-{}-herdr-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let herdr = dir.join("herdr");
    let log = dir.join("args.log");
    let script = format!("#!/bin/sh\necho \"$*\" >> '{}'\n{body}\n", log.display());
    std::fs::write(&herdr, script).unwrap();
    std::fs::set_permissions(&herdr, std::fs::Permissions::from_mode(0o755)).unwrap();
    Fake { dir, remote: Remote::new(herdr) }
}

const CLI: &str = r#"
case "$*" in
  "--machine hub agent list") echo '{"id":"cli:agent:list","result":{"type":"agent_list","agents":[{"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","terminal_id":"t1","agent":"claude","agent_status":"blocked","focused":false,"revision":2,"state_change_seq":7}]}}' ;;
  "--machine hub agent read"*) printf 'line one\nline two\n' ;;
  "--machine hub agent focus"*) echo '{"id":"cli:agent:focus","result":{"type":"ok"}}' ;;
  "machine list --json") echo '[{"id":"a","label":"hub","enabled":true},{"id":"b","label":"off","enabled":false},{"id":"c","label":"mini","enabled":true}]' ;;
esac
"#;

#[test]
fn list_read_focus_and_machines_run_the_cli() {
    let f = fake("calls", CLI);
    let agents = f.remote.list("hub").unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!((agents[0].machine.as_str(), agents[0].status), ("hub", model::Status::Blocked));
    assert_eq!(agents[0].seq, 7);
    assert_eq!(f.remote.read("hub", "w1:p1", 40).unwrap(), "line one\nline two\n");
    f.remote.focus("hub", "w1:p1").unwrap();
    assert_eq!(f.remote.machines().unwrap(), ["hub", "mini"]);
    assert_eq!(
        f.args(),
        [
            "--machine hub agent list",
            "--machine hub agent read w1:p1 --source recent-unwrapped --lines 40",
            "--machine hub agent focus w1:p1",
            "machine list --json",
        ]
    );
}

#[test]
fn errors_name_the_machine_and_the_cause() {
    let f = fake(
        "errors",
        r#"
case "$2" in
  api) echo '{"error":{"code":"agent_not_found","message":"agent target w9:p9 not found"},"id":"cli:agent:read"}' >&2; exit 1 ;;
  bogus) echo "error: unknown machine 'bogus'; use \`herdr machine list\`" >&2; exit 2 ;;
  quiet) exit 3 ;;
  garbled) echo 'not json'; exit 0 ;;
esac
echo 'not json'
"#,
    );
    assert_eq!(f.remote.read("api", "w9:p9", 1).unwrap_err(), "api: agent target w9:p9 not found");
    assert_eq!(
        f.remote.list("bogus").unwrap_err(),
        "bogus: unknown machine 'bogus'; use `herdr machine list`"
    );
    assert_eq!(
        f.remote.focus("quiet", "x").unwrap_err(),
        "quiet: herdr exited with exit status: 3"
    );
    assert!(f.remote.list("garbled").unwrap_err().starts_with("garbled: bad herdr reply"));
    assert!(f.remote.machines().unwrap_err().starts_with("herdr machine list: "));
}

#[test]
fn a_slow_call_is_killed_at_the_timeout() {
    let f = fake("slow", "exec sleep 5");
    let remote = f.remote.clone().with_timeout(Duration::from_millis(100));
    let start = Instant::now();
    assert_eq!(remote.list("hub").unwrap_err(), "hub: timed out after 0.1 s");
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_missing_herdr_is_an_error() {
    let remote = Remote::new("/nonexistent/herdr");
    assert_eq!(remote.herdr(), Path::new("/nonexistent/herdr"));
    assert_eq!(remote.list("hub").unwrap_err(), "hub: /nonexistent/herdr not found");
    let dir = Remote::new("/");
    assert!(dir.machines().unwrap_err().starts_with("herdr: /: "));
}

#[test]
fn the_child_loses_herdrs_pane_variables() {
    let cmd = Remote::new("herdr").command(&["agent", "list"]);
    let removed: Vec<_> = cmd
        .get_envs()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    for name in HERDR_ENV {
        assert!(removed.iter().any(|r| r == name), "{name} kept: {removed:?}");
    }
    assert!(removed.iter().all(|r| r.starts_with("HERDR_")), "{removed:?}");
}

#[test]
fn any_other_herdr_variable_is_removed_too() {
    let mut cmd = Command::new("herdr");
    strip_herdr_env(&mut cmd, ["HERDR_NEW_THING", "PATH"].map(OsString::from));
    let removed: Vec<_> = cmd.get_envs().map(|(k, _)| k.to_string_lossy().into_owned()).collect();
    assert!(removed.iter().any(|r| r == "HERDR_NEW_THING"), "{removed:?}");
    assert!(!removed.iter().any(|r| r == "PATH"), "{removed:?}");
}

#[test]
fn resolve_finds_herdr_on_path_or_takes_a_path() {
    assert_eq!(resolve("/usr/local/bin/herdr"), PathBuf::from("/usr/local/bin/herdr"));
    let home = dirs::home_dir().unwrap();
    assert_eq!(resolve("~/bin/herdr"), home.join("bin/herdr"));
    // `sh` is on PATH everywhere the tests run.
    let sh = resolve("sh");
    assert!(sh.is_absolute() && sh.ends_with("sh"), "{}", sh.display());
    assert_eq!(resolve("no-such-herdr-xyz"), PathBuf::from("no-such-herdr-xyz"));
}

#[test]
fn cli_errors_take_the_first_line() {
    assert_eq!(cli_error(""), None);
    assert_eq!(cli_error("\n  \nerror: boom\nmore"), Some("boom".into()));
    assert_eq!(cli_error("{\"ok\":1}"), Some("{\"ok\":1}".into()));
}
