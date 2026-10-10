use super::*;
use crate::modules::sys::check::Status;
use crate::modules::sys::settings::{Domain, Target};

fn service(name: &str, kind: Kind, target: &str) -> Service {
    let target = Target::One(target.into());
    Service { name: name.into(), kind, target, warn: None, fail: None, log: None, restart: false, domain: Domain::Gui }
}

#[test]
fn the_argv_never_prompts_and_reads_the_script_from_stdin() {
    assert_eq!(
        argv("jaymin@pro"),
        ["/usr/bin/ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "jaymin@pro", "/bin/sh", "-s"]
    );
}

#[test]
fn the_script_is_the_probe_plus_quoted_remote_checks() {
    let services = [
        service("web", Kind::Http, "http://pro:11434/"),
        service("ollama", Kind::Launchd, "com.ollama.ollama"),
        service("odd", Kind::Process, "it's$(x)"),
    ];
    let s = script(&services);
    assert!(s.starts_with(PROBE), "{s}");
    let rest: Vec<&str> = s[PROBE.len()..].lines().collect();
    assert_eq!(rest, [
        "echo '@@ uid'",
        "/usr/bin/id -u",
        r#"out=$(/bin/launchctl print "gui/$(/usr/bin/id -u)/"'com.ollama.ollama' 2>&1); rc=$?; printf '@@ svc 1\n%s\n@@ rc 1 %s\n' "$out" "$rc""#,
        r#"out=$(/usr/bin/pgrep -x 'it'\''s$(x)' 2>&1); rc=$?; printf '@@ svc 2\n%s\n@@ rc 2 %s\n' "$out" "$rc""#,
    ]);
    assert!(remote(&services[1]) && !remote(&services[0]));
}

#[test]
fn verdicts_come_from_each_check_section() {
    let services = [
        service("web", Kind::Tcp, "pro:1"),
        service("ollama", Kind::Launchd, "com.ollama.ollama"),
        service("gone", Kind::Launchd, "gone"),
        service("sync", Kind::Process, "syncthing"),
        service("lost", Kind::Process, "lost"),
    ];
    let running = include_str!("../fixtures/launchctl_running.txt");
    let missing = include_str!("../fixtures/launchctl_missing.txt");
    let out = format!(
        "@@ host\npro\n@@ uid\n502\n@@ svc 1\n{running}@@ rc 1 0\n@@ svc 2\n{missing}@@ rc 2 113\n@@ svc 3\n917\n@@ rc 3 0\n"
    );
    let v = verdicts(&out, &services);
    assert_eq!(v[0], None);
    let status = |i: usize| v[i].as_ref().map(|v| (v.status, v.reason.clone()));
    assert_eq!(status(1).unwrap().0, Status::Ok);
    assert_eq!(status(2), Some((Status::Fail, "not loaded in gui/502".into())));
    assert_eq!(status(3), Some((Status::Ok, "running (pid 917)".into())));
    // The ssh call ended before this check printed its exit.
    assert_eq!(status(4), Some((Status::Unknown, "no answer over ssh".into())));
    // Without the uid section the domain is unknown.
    let v = verdicts(&format!("@@ svc 0\n{missing}@@ rc 0 113\n"), &services[2..3]);
    assert_eq!(v[0].as_ref().unwrap().reason, "not loaded in gui/?");
}

#[test]
fn a_system_daemon_is_read_in_the_system_domain() {
    let daemon = Service { domain: Domain::System, ..service("d", Kind::Launchd, "com.d") };
    let rest = script(std::slice::from_ref(&daemon));
    assert!(rest.contains(r#"out=$(/bin/launchctl print "system/"'com.d' 2>&1)"#), "{rest}");
    let missing = include_str!("../fixtures/launchctl_missing.txt");
    let v = verdicts(&format!("@@ svc 0\n{missing}@@ rc 0 113\n"), &[daemon]);
    assert_eq!(v[0].as_ref().unwrap().reason, "not loaded in system");
}

#[test]
fn sections_end_at_the_next_marker() {
    let out = "@@ svc 1\na\n@@ svc 10\nb\nc\n@@ rc 10 0\n";
    assert_eq!(section(out, "svc 1"), "a\n");
    assert_eq!(section(out, "svc 10"), "b\nc\n");
    assert_eq!(section(out, "svc 2"), "");
}

#[test]
fn ssh_failures_say_why() {
    let exit = |code, stdout: &str, stderr: &str| Exit { code, stdout: stdout.into(), stderr: stderr.into() };
    let refused = exit(Some(255), "", "\nssh: connect to host pro port 22: Connection refused\n");
    assert_eq!(failure(&refused).unwrap(), "ssh: ssh: connect to host pro port 22: Connection refused");
    assert_eq!(failure(&exit(Some(255), "partial", "")).unwrap(), "ssh: exit 255");
    assert_eq!(failure(&exit(Some(0), " \n", "")).unwrap(), "ssh: exit 0");
    assert_eq!(failure(&exit(None, "", "")).unwrap(), "ssh: killed by a signal");
    assert_eq!(failure(&exit(Some(1), "@@ host\npro\n", "warning")), None);
}
