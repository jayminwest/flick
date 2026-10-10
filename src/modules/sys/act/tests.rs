use super::*;
use crate::config::parse;
use crate::modules::sys::io::Entry;
use crate::modules::sys::settings::Settings;

const CONFIG: &str = r#"
[[sys.service]]
name = "agent"
kind = "launchd"
target = "dev.agent"
log = "~/Library/Logs/agent.log"
restart = true
[[sys.service]]
name = "web"
kind = "http"
target = "http://ok/"
log = "/var/log/web.log"
[[sys.service]]
name = "odd"
kind = "process"
target = "odd"
log = "logs/odd.log"
[[sys.machine]]
name = "laptop"
via = "local"
vnc = "vnc://laptop"
[[sys.machine]]
name = "server"
via = "flick"
ssh = "me@server"
dash = "https://server:8310/"
[[sys.machine.service]]
name = "pundit"
kind = "launchd"
target = "dev.pundit's"
log = "~/logs/it's.log"
restart = true
[[sys.machine]]
name = "bare"
via = "flick"
[[sys.machine]]
name = "pro"
via = "ssh"
ssh = "pro"
[[sys.machine.service]]
name = "ollama"
kind = "process"
target = "ollama"
log = "/opt/ollama.log"
"#;

fn setup() -> (Fleet, Vec<Entry>) {
    let s: Settings = parse(CONFIG).unwrap().section(ID).unwrap().unwrap().get::<Settings>().unwrap().check().unwrap();
    let mut fleet = Fleet::default();
    fleet.set_machines(&s.machine);
    let mine = s.service.into_iter().map(|service| Entry { service, verdict: None, checked_at: None }).collect();
    (fleet, mine)
}

fn target(machine: Option<&str>, service: &str) -> Result<Target, String> {
    let (fleet, mine) = setup();
    resolve(&fleet, &mine, machine, service)
}

#[test]
fn resolves_only_services_this_macs_config_defines() {
    let here = target(None, "agent").unwrap();
    assert_eq!((here.machine.as_str(), here.place.clone()), (HERE, Place::Local));
    let local = target(Some("laptop"), "agent").unwrap();
    assert_eq!((local.machine.as_str(), local.service.name.as_str()), ("laptop", "agent"));
    let server = target(Some("server"), "pundit").unwrap();
    assert_eq!(server.place, Place::Ssh("me@server".into()));
    assert_eq!(server.what(), "pundit on server");
    assert_eq!(target(Some("nope"), "x").unwrap_err(), "sys: no machine \"nope\"");
    assert_eq!(target(None, "pundit").unwrap_err(), "sys: no service \"pundit\" on this Mac in this Mac's config");
    assert_eq!(target(Some("bare"), "x").unwrap_err(), "sys: no service \"x\" on bare in this Mac's config");
    // A machine without ssh cannot list services (settings.rs); one with ssh but no target
    // in config is impossible, so the error is only for a hand-built fleet.
    let (mut fleet, mine) = setup();
    fleet.slots[1].machine.ssh = None;
    assert_eq!(resolve(&fleet, &mine, Some("server"), "pundit").unwrap_err(), "sys: server has no ssh target");
}

#[test]
fn restart_is_launchctl_kickstart_here_or_over_ssh() {
    let here = target(None, "agent").unwrap();
    let run = here.restart(Some(501)).unwrap();
    assert_eq!(run.argv, ["/bin/launchctl", "kickstart", "-k", "gui/501/dev.agent"]);
    assert_eq!((run.input, run.shown.as_str()), (None, "launchctl kickstart -k gui/501/dev.agent"));
    assert_eq!(here.restart(None).unwrap_err(), "sys: no uid for the launchd domain");
    let ssh = target(Some("server"), "pundit").unwrap().restart(None).unwrap();
    assert_eq!(ssh.argv, ssh::argv("me@server"));
    assert_eq!(ssh.input.as_deref(), Some("exec /bin/launchctl kickstart -k \"gui/$(/usr/bin/id -u)/\"'dev.pundit'\\''s'\n"));
    assert_eq!(ssh.shown, "ssh me@server launchctl kickstart -k gui/$(id -u)/dev.pundit's");
    assert_eq!(
        target(None, "web").unwrap().restart(Some(501)).unwrap_err(),
        "sys: web on this Mac may not be restarted (set restart = true on a launchd service)"
    );
}

#[test]
fn tail_reads_the_configured_log() {
    let run = target(None, "agent").unwrap().tail("/Users/me/").unwrap();
    assert_eq!(run.argv, ["/usr/bin/tail", "-n", "100", "--", "/Users/me/Library/Logs/agent.log"]);
    assert_eq!((run.input, run.shown.as_str()), (None, "tail -n 100 /Users/me/Library/Logs/agent.log"));
    let abs = target(None, "web").unwrap().tail("/h").unwrap();
    assert_eq!(abs.argv[4], "/var/log/web.log");
    let ssh = target(Some("server"), "pundit").unwrap().tail("/h").unwrap();
    assert_eq!(ssh.argv, ssh::argv("me@server"));
    assert_eq!(ssh.input.as_deref(), Some("exec /usr/bin/tail -n 100 -- \"$HOME\"/'logs/it'\\''s.log'\n"));
    assert_eq!(ssh.shown, "ssh me@server tail -n 100 ~/logs/it's.log");
    let pro = target(Some("pro"), "ollama").unwrap().tail("/h").unwrap();
    assert_eq!(pro.input.as_deref(), Some("exec /usr/bin/tail -n 100 -- '/opt/ollama.log'\n"));
    assert_eq!(target(None, "odd").unwrap().tail("/h").unwrap_err(), "sys: the log of odd on this Mac is a path from / or ~/");
    let mut none = target(None, "web").unwrap();
    none.service.log = Some("  ".into());
    assert_eq!(none.tail("/h").unwrap_err(), "sys: web on this Mac has no log (set log = \"<path>\")");
}

#[test]
fn the_confirm_shows_the_exact_command_and_round_trips_its_token() {
    let t = target(Some("server"), "pundit").unwrap();
    let c = t.confirm(&t.restart(None).unwrap());
    assert!(c.destructive);
    assert_eq!((c.module, c.title.as_str(), c.label.as_str()), (ID, "Restart pundit on server?", "Restart pundit"));
    assert_eq!(c.rows.len(), 1);
    assert_eq!(c.rows[0].title, "ssh me@server launchctl kickstart -k gui/$(id -u)/dev.pundit's");
    assert_eq!((c.rows[0].subtitle.as_str(), c.rows[0].accessory.as_str()), ("runs over ssh as me@server", "launchd"));
    assert_eq!(parse_token(&c.token), Some(("server", "pundit")));
    let here = target(Some("laptop"), "agent").unwrap();
    assert_eq!(here.confirm(&here.restart(Some(1)).unwrap()).rows[0].subtitle, "runs on this Mac");
    for bad in ["", "restart", "restart\tm", "delete\tm\ts"] {
        assert_eq!(parse_token(bad), None, "{bad:?}");
    }
    assert_eq!(parse_token("restart\ta/b\tc\td"), Some(("a/b", "c\td")));
}

#[expect(clippy::unnecessary_wraps, reason = "runs answer as Result")]
fn exit(code: Option<i32>, stdout: &str, stderr: &str) -> Result<Exit, String> {
    Ok(Exit { code, stdout: stdout.into(), stderr: stderr.into() })
}

#[test]
fn results_say_what_happened() {
    assert_eq!(restarted(exit(Some(0), "", ""), "a on b"), Ok("Restarted a on b".into()));
    let missing = exit(Some(113), "", "\nCould not find service \"x\"\n");
    assert_eq!(restarted(missing, "a on b"), Err("Restart a on b failed: Could not find service \"x\"".into()));
    assert_eq!(restarted(exit(Some(255), "", ""), "a"), Err("Restart a failed: exit 255".into()));
    assert_eq!(restarted(exit(None, "", ""), "a"), Err("Restart a failed: killed by a signal".into()));
    assert_eq!(restarted(Err("timed out after 10 s".into()), "a"), Err("Restart a failed: timed out after 10 s".into()));
    assert_eq!(tailed(exit(Some(0), "l1\nl2\n", "")), Ok("l1\nl2\n".into()));
    assert_eq!(tailed(exit(Some(1), "", "tail: x: No such file or directory\n")), Err("tail: x: No such file or directory".into()));
    assert_eq!(tailed(Err("ssh: refused".into())), Err("ssh: refused".into()));
}

fn keys(actions: &[Action]) -> Vec<&str> {
    actions.iter().map(|a| a.key).collect()
}

#[test]
fn menus_offer_only_what_config_allows() {
    let (fleet, _) = setup();
    let machine = |i: usize| keys(&machine_menu(&fleet.slots[i].machine)).join(",");
    assert_eq!([machine(0), machine(1), machine(2)], ["vnc", "dash", ""]);
    let titles: Vec<String> = machine_menu(&fleet.slots[0].machine).into_iter().map(|a| a.title).collect();
    assert_eq!(titles, ["Screen Sharing"]);
    assert_eq!(keys(&service_menu(&target(None, "agent").unwrap())), ["tail", "restart"]);
    assert_eq!(keys(&service_menu(&target(None, "web").unwrap())), ["tail"]);
    let mut quiet = target(None, "web").unwrap();
    quiet.service.log = None;
    assert!(service_menu(&quiet).is_empty());
}
