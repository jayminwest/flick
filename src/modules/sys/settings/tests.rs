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
