use super::*;
use crate::config::example::assert_documents;
use crate::config::parse;

fn settings(text: &str) -> Result<Settings, String> {
    parse(text)?.section("sys")?.expect("enabled").get::<Settings>()?.check()
}

fn one(fields: &str) -> Result<Settings, String> {
    settings(&format!("[[sys.service]]\nname = \"x\"\n{fields}"))
}

#[test]
fn the_example_documents_every_key() {
    assert_documents::<Settings>("sys");
    assert_documents::<Service>("sys.service");
    assert_documents::<Machine>("sys.machine");
    assert_documents::<Service>("sys.machine.service");
}

#[test]
fn defaults_check_nothing() {
    assert_eq!(settings("").unwrap(), Settings::default());
    assert!(settings("[sys]\nenabled = true").unwrap().service.is_empty());
    assert!(settings("[sys]\nevery = 3").unwrap_err().starts_with("[sys]: unknown field `every`"));
}

#[test]
fn reads_each_kind() {
    let text = r#"
[[sys.service]]
name = "memory"
kind = "http"
target = "http://127.0.0.1:8300/health"
warn = 200
[[sys.service]]
name = "ollama"
kind = "tcp"
target = "127.0.0.1:11434"
[[sys.service]]
name = "life-ops"
kind = "launchd"
target = "org.nix-community.home.kota-life-ops"
log = "~/Library/Logs/kota-life-ops.log"
restart = true
[[sys.service]]
name = "syncthing"
kind = "process"
target = "syncthing"
[[sys.service]]
name = "backlog"
kind = "command"
target = ["/bin/sh", "-c", "wc -l < pending.jsonl"]
warn = 10
fail = 50
"#;
    let s = settings(text).unwrap().service;
    let kinds: Vec<&str> = s.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(kinds, ["http", "tcp", "launchd", "process", "command"]);
    assert_eq!(s[0].limits(), Limits { warn: Some(200.0), fail: None });
    assert_eq!(s[2].log.as_deref(), Some("~/Library/Logs/kota-life-ops.log"));
    assert!(s[2].restart && !s[0].restart);
    assert_eq!(s[1].word(), "127.0.0.1:11434");
    assert_eq!(s[4].word(), "");
    assert_eq!(s[4].argv(), ["/bin/sh", "-c", "wc -l < pending.jsonl"]);
    assert_eq!(s[3].argv(), ["syncthing"]);
    // A command shows only its program: arguments may hold a token.
    assert_eq!(s[4].shown_target(), "/bin/sh");
    assert_eq!(s[0].shown_target(), "http://127.0.0.1:8300/health");
}

#[test]
fn rejects_bad_services() {
    let argv = "command target is an argv, e.g. [\"/bin/echo\", \"1\"]";
    let word = "target is one word without / or spaces";
    let limits = "warn and fail apply to http, tcp and command";
    let cases = [
        ("kind = \"http\"\ntarget = \"ftp://x\"", "http target must start with http:// or https://"),
        ("kind = \"tcp\"\ntarget = \"localhost\"", "tcp target is host:port"),
        ("kind = \"tcp\"\ntarget = \":80\"", "tcp target is host:port"),
        ("kind = \"tcp\"\ntarget = \"h:99999\"", "tcp target is host:port"),
        ("kind = \"launchd\"\ntarget = \"gui/501/x\"", word),
        ("kind = \"process\"\ntarget = \"a b\"", word),
        ("kind = \"process\"\ntarget = \" \"", "target is empty"),
        ("kind = \"process\"\ntarget = [\"x\"]", "target is a string for this kind"),
        ("kind = \"command\"\ntarget = \"echo 1\"", argv),
        ("kind = \"command\"\ntarget = []", argv),
        ("kind = \"command\"\ntarget = [\" \"]", argv),
        ("kind = \"launchd\"\ntarget = \"x\"\nwarn = 1", limits),
        ("kind = \"process\"\ntarget = \"x\"\nfail = 1", limits),
        ("kind = \"tcp\"\ntarget = \"h:1\"\nrestart = true", "restart = true needs kind = \"launchd\""),
        ("kind = \"tcp\"\ntarget = \"h:1\"\nwarn = nan", "warn and fail are numbers"),
    ];
    for (fields, why) in cases {
        assert_eq!(one(fields).unwrap_err(), format!("[sys]: service \"x\": {why}"), "{fields}");
    }
}

#[test]
fn serde_rejects_unknown_kinds_and_keys() {
    let err = |fields: &str| one(fields).unwrap_err();
    assert!(err("kind = \"smtp\"\ntarget = \"x\"").starts_with("[sys]: unknown variant `smtp`"));
    assert!(err("kind = \"tcp\"\ntarget = \"h:1\"\nretries = 2").starts_with("[sys]: unknown field `retries`"));
    assert!(one("kind = \"tcp\"\ntarget = \"[::1]:22\"").is_ok());
    assert!(one("kind = \"http\"\ntarget = \"https://x\"\nwarn = 1\nfail = 2").is_ok());
}

#[test]
fn names_are_present_and_unique() {
    let svc = |name: &str| format!("[[sys.service]]\nname = \"{name}\"\nkind = \"process\"\ntarget = \"x\"\n");
    assert_eq!(settings(&svc(" ")).unwrap_err(), "[sys]: service 1 has no name");
    let twice = format!("{}{}", svc("a"), svc("a"));
    assert_eq!(settings(&twice).unwrap_err(), "[sys]: service \"a\" is named twice");
    assert_eq!(settings(&format!("{}{}", svc("a"), svc("b"))).unwrap().service.len(), 2);
}

fn machine(fields: &str) -> Result<Settings, String> {
    settings(&format!("[[sys.machine]]\nname = \"m\"\n{fields}"))
}

#[test]
fn reads_machines_and_the_refresh() {
    let text = r#"
[sys]
refresh_secs = 60
[[sys.machine]]
name = "laptop"
via = "local"
[[sys.machine]]
name = "mbp-server"
via = "flick"
ssh = "jaymin@mbp-server"
dash = "https://mbp-server.ts.net:8310/"
[[sys.machine]]
name = "mac-pro"
via = "ssh"
host = "pro:7419"
ssh = "jaymin@100.118.223.57"
vnc = "vnc://100.118.223.57"
[[sys.machine.service]]
name = "ollama"
kind = "launchd"
target = "com.ollama.ollama"
[[sys.machine.service]]
name = "mlx"
kind = "tcp"
target = "100.118.223.57:11234"
"#;
    let s = settings(text).unwrap();
    assert_eq!(s.refresh_secs, 60);
    let vias: Vec<&str> = s.machine.iter().map(|m| m.via.as_str()).collect();
    assert_eq!(vias, ["local", "flick", "ssh"]);
    assert_eq!((s.machine[1].flick_host(), s.machine[2].flick_host()), ("mbp-server", "pro:7419"));
    let names: Vec<&str> = s.machine[2].service.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["ollama", "mlx"]);
    assert!(settings("").unwrap().machine.is_empty() && settings("").unwrap().refresh_secs == 0);
}

#[test]
fn rejects_bad_machines() {
    let svc = |kind: &str, target: &str| format!("via = \"ssh\"\nssh = \"h\"\n[[sys.machine.service]]\nname = \"s\"\nkind = \"{kind}\"\ntarget = {target}");
    let cases = [
        ("via = \"ssh\"".to_string(), "via = \"ssh\" needs ssh = \"user@host\""),
        ("via = \"ssh\"\nssh = \"-oProxyCommand=x\"".into(), "ssh is one word, e.g. \"user@host\", not an option"),
        ("via = \"ssh\"\nssh = \"a b\"".into(), "ssh is one word, e.g. \"user@host\", not an option"),
        ("via = \"flick\"\nhost = \" \"".into(), "host is one word, name[:port]"),
        ("via = \"flick\"\nvnc = \"http://x\"".into(), "vnc must start with vnc://"),
        ("via = \"flick\"\ndash = \"vnc://x\"".into(), "dash must start with http:// or https://"),
        (svc("command", "[\"/bin/echo\"]"), "a command check runs on the machine's own Flick, not over ssh"),
        (svc("tcp", "\"nohost\""), "service \"s\": tcp target is host:port"),
        (format!("{}\n[[sys.machine.service]]\nname = \"s\"\nkind = \"process\"\ntarget = \"y\"", svc("process", "\"x\"")), "service \"s\" is named twice"),
        ("via = \"local\"\nssh = \"h\"\n[[sys.machine.service]]\nname = \"s\"\nkind = \"process\"\ntarget = \"x\"".into(), "services belong to machines with an ssh target; this Mac's are [[sys.service]]"),
        ("via = \"flick\"\n[[sys.machine.service]]\nname = \"s\"\nkind = \"process\"\ntarget = \"x\"".into(), "services belong to machines with an ssh target; this Mac's are [[sys.service]]"),
    ];
    for (fields, why) in cases {
        assert_eq!(machine(&fields).unwrap_err(), format!("[sys]: machine \"m\": {why}"), "{fields}");
    }
    let named = |name: &str| format!("[[sys.machine]]\nname = \"{name}\"\nvia = \"local\"\n");
    assert_eq!(settings(&named("")).unwrap_err(), "[sys]: machine 1 has no name");
    assert_eq!(settings(&format!("{}{}", named("a"), named("a"))).unwrap_err(), "[sys]: machine \"a\" is named twice");
    assert!(machine("via = \"telnet\"").unwrap_err().starts_with("[sys]: unknown variant `telnet`"));
    for (secs, ok) in [(0, true), (29, false), (30, true), (3600, true)] {
        let got = settings(&format!("[sys]\nrefresh_secs = {secs}"));
        assert_eq!(got.is_ok(), ok, "{secs}");
    }
    assert_eq!(settings("[sys]\nrefresh_secs = 5").unwrap_err(), "[sys]: refresh_secs is 0 (off) or at least 30");
}
