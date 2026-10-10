//! Control socket replies: scripts parse them. Text answers stay `{"ok":"<text>"}` with or
//! without `--json`; a module that reads `Cx::json` and answers a JSON object or array gets
//! `{"ok":<value>}`; errors are `{"error":"<message>"}`. A `--remote` word before `--json`
//! sets `Cx::remote` and changes nothing for modules that ignore it. Over the network
//! (`flick --host`) every request is a remote caller's, and side-effecting requests are
//! refused before any module sees them.

use crate::config::Config;
use crate::control::net::gate;
use crate::control::server::EVENTS_REFUSED;
use crate::core::control::{Flags, Reply, net_policy, split_flags};
use crate::core::{Cx, Module, Registry, test_cx};
use crate::modules::{Clips, with_apps};

/// A module that answers JSON when the request asked for it.
struct Report;

impl Module for Report {
    fn id(&self) -> &'static str {
        "report"
    }

    fn command(&mut self, _args: &[String], cx: &mut Cx) -> Result<String, String> {
        Ok(if cx.json { r#"{"apps": ["Zed"], "secs": 60}"#.into() } else { "Zed 1m".into() })
    }
}

/// A module that refuses remote callers.
struct Private;

impl Module for Private {
    fn id(&self) -> &'static str {
        "private"
    }

    fn command(&mut self, _args: &[String], cx: &mut Cx) -> Result<String, String> {
        if cx.remote {
            return Err("private: remote use not permitted".into());
        }
        Ok(if cx.json { r#"{"n":1}"#.into() } else { "n 1".into() })
    }
}

/// The reply line for request `words`, as the control socket builds it.
fn reply(registry: &mut Registry, words: &[&str], cx: &mut Cx) -> String {
    let (words, flags) = split_flags(words.iter().map(|w| (*w).to_string()).collect());
    cx.json = flags.json;
    cx.remote = flags.remote;
    let result = registry.command(&words, cx);
    Reply::answer(result, flags.json).to_line()
}

#[test]
fn text_replies_keep_their_shape_with_or_without_json() {
    let mut registry = with_apps(&Config::default(), vec![]).unwrap();
    test_cx("", |cx| {
        registry.migrate(cx.store).unwrap();
        cx.store.add_clip("[1, 2]");
        let id = cx.store.clips()[0].id;
        assert_eq!(
            reply(&mut registry, &["clip", "list"], cx),
            format!(r#"{{"ok":"{id}\t[1, 2]"}}"#)
        );
        assert_eq!(
            reply(&mut registry, &["clip", "list", "--json"], cx),
            format!(r#"{{"ok":"{id}\t[1, 2]"}}"#)
        );
        assert_eq!(
            reply(&mut registry, &["clip", "get", &id.to_string()], cx),
            r#"{"ok":"[1, 2]"}"#
        );
        assert_eq!(
            reply(&mut registry, &["clip", "nope", "--json"], cx),
            r#"{"error":"clip: unknown command \"nope\""}"#
        );
        let unknown = reply(&mut registry, &["nope", "--json"], cx);
        assert!(unknown.starts_with(r#"{"error":"unknown module \"nope\" "#), "{unknown}");
    });
}

#[test]
fn json_answers_are_embedded_as_values() {
    let mut registry = Registry::new(vec![Box::new(Report)]);
    test_cx("", |cx| {
        assert_eq!(reply(&mut registry, &["report", "today"], cx), r#"{"ok":"Zed 1m"}"#);
        assert_eq!(
            reply(&mut registry, &["report", "today", "--json"], cx),
            r#"{"ok":{"apps":["Zed"],"secs":60}}"#
        );
        // Only a trailing --json asks; elsewhere it is an argument.
        assert_eq!(reply(&mut registry, &["report", "--json", "today"], cx), r#"{"ok":"Zed 1m"}"#);
    });
}

#[test]
fn capture_ls_json_is_an_array() {
    let mut registry = with_apps(&Config::default(), vec![]).unwrap();
    test_cx("", |cx| {
        registry.migrate(cx.store).unwrap();
        assert_eq!(reply(&mut registry, &["capture", "ls", "--json"], cx), r#"{"ok":[]}"#);
        assert_eq!(reply(&mut registry, &["capture", "ls"], cx), r#"{"ok":""}"#);
        assert_eq!(
            reply(&mut registry, &["capture", "last", "--json"], cx),
            r#"{"error":"capture: no captures yet"}"#
        );
    });
}

#[test]
fn remote_requests_answer_like_local_ones_unless_a_module_checks() {
    let mut registry = with_apps(&Config::default(), vec![]).unwrap();
    test_cx("", |cx| {
        registry.migrate(cx.store).unwrap();
        cx.store.add_clip("x");
        // Modules that ignore Cx::remote: identical replies with and without --remote.
        for request in [&["clip", "list"][..], &["capture", "ls"], &["clip", "nope"]] {
            let local = reply(&mut registry, request, cx);
            let remote = reply(&mut registry, &[request, &["--remote"]].concat(), cx);
            assert_eq!(remote, local, "{request:?}");
            let local = reply(&mut registry, &[request, &["--json"]].concat(), cx);
            let remote = reply(&mut registry, &[request, &["--remote", "--json"]].concat(), cx);
            assert_eq!(remote, local, "{request:?} --json");
        }
        assert_eq!(
            reply(&mut registry, &["capture", "ls", "--remote", "--json"], cx),
            r#"{"ok":[]}"#
        );
        // Activity checks: without the user's grant an agent reads nothing and grants nothing.
        let refused = r#"{"error":"activity: remote use not permitted; the user can run `flick activity remote allow` or choose 'Allow Agents to Read Activity' in Flick"}"#;
        assert_eq!(reply(&mut registry, &["activity", "today", "--remote", "--json"], cx), refused);
        assert_eq!(
            reply(&mut registry, &["activity", "remote", "allow", "--remote", "--json"], cx),
            r#"{"error":"activity: not allowed from an agent session (--remote)"}"#
        );
        assert!(
            reply(&mut registry, &["activity", "today", "--json"], cx).starts_with(r#"{"ok":"#)
        );
    });
}

#[test]
fn a_module_that_checks_remote_sees_only_the_trailing_word() {
    let mut registry = Registry::new(vec![Box::new(Private)]);
    test_cx("", |cx| {
        assert_eq!(reply(&mut registry, &["private", "n"], cx), r#"{"ok":"n 1"}"#);
        assert_eq!(reply(&mut registry, &["private", "n", "--json"], cx), r#"{"ok":{"n":1}}"#);
        let refused = r#"{"error":"private: remote use not permitted"}"#;
        assert_eq!(reply(&mut registry, &["private", "n", "--remote"], cx), refused);
        assert_eq!(reply(&mut registry, &["private", "n", "--remote", "--json"], cx), refused);
        // Elsewhere --remote is an argument, and --json before it is too.
        assert_eq!(reply(&mut registry, &["private", "--remote", "n"], cx), r#"{"ok":"n 1"}"#);
        assert_eq!(reply(&mut registry, &["private", "n", "--json", "--remote"], cx), refused);
    });
}

/// The reply line for `words` from the real registry, run as `run` would on the main thread.
#[expect(clippy::needless_pass_by_value, reason = "matches the transport's handler type")]
fn run(words: Vec<String>, flags: Flags) -> Reply {
    let mut registry = with_apps(&Config::default(), vec![]).unwrap();
    test_cx("", |cx| {
        registry.migrate(cx.store).unwrap();
        cx.json = flags.json;
        cx.remote = flags.remote;
        Reply::answer(registry.command(&words, cx), flags.json)
    })
}

/// The reply line for request `words` sent over the network transport.
fn net_reply(words: &[&str]) -> String {
    gate(words.iter().map(|w| (*w).to_string()).collect(), run).to_line()
}

#[test]
fn network_callers_are_refused_side_effecting_requests() {
    let refused = [
        (&["reload"][..], "reload"),
        (&["flick", "rebuild"], "flick rebuild"),
        (&["flick", "cancel"], "flick cancel"),
        (&["remote", "on"], "remote on"),
        (&["remote", "off"], "remote off"),
        (&["remote"], "remote"),
        (&["keys", "fire", "x"], "keys fire"),
        (&["app", "uninstall", "/Applications/Zed.app"], "app uninstall"),
        (&["quicklink", "add", "x", "https://x"], "quicklink add"),
        (&["quicklink", "remove", "x"], "quicklink remove"),
        (&["capture", "ls"], "capture ls"),
        (&["capture", "screen"], "capture screen"),
        (&["capture"], "capture"),
        (&["feedback", "resolve", "x"], "feedback resolve"),
        (&["task", "rm", "3"], "task rm"),
        (&["script", "run", "Lock"], "script run"),
        (&["message", "card", "press", "c1", "ok"], "message card press"),
        (&["message", "card", "focus"], "message card focus"),
    ];
    for (words, what) in refused {
        let want = format!(r#"{{"error":"{what}: not allowed over the network"}}"#);
        assert_eq!(net_reply(words), want, "{words:?}");
        assert_eq!(net_reply(&[words, &["--json"]].concat()), want, "{words:?} --json");
    }
    // `["events"]` is refused by the transport unless `[remote] events = true`.
    assert_eq!(
        Reply::Error(EVENTS_REFUSED.into()).to_line(),
        r#"{"error":"events: not allowed over the network"}"#
    );
}

#[test]
fn network_requests_are_remote_even_without_the_flag() {
    // Activity checks Cx::remote: a network caller is refused without --remote.
    let refused = r#"{"error":"activity: remote use not permitted; the user can run `flick activity remote allow` or choose 'Allow Agents to Read Activity' in Flick"}"#;
    assert_eq!(net_reply(&["activity", "today", "--json"]), refused);
    assert_eq!(net_reply(&["activity", "today"]), refused);
    assert!(
        run(vec!["activity".into(), "today".into()], Flags::default())
            .to_line()
            .starts_with(r#"{"ok":"#)
    );
    // Requests outside the deny table answer as for a local remote caller.
    assert_eq!(
        net_reply(&["capture", "ls"]),
        r#"{"error":"capture ls: not allowed over the network"}"#
    );
    assert!(net_reply(&["task", "ls"]).starts_with(r#"{"ok":"#));
    // Posting a message is what peers are for (`docs/message.md`). Not run here: a post
    // shows the panel, which needs AppKit's main thread.
    let post = ["message", "post", "--reply-to", "k1", "hi"].map(String::from);
    assert_eq!(net_policy(&post), Ok(()));
    assert_eq!(net_reply(&["message", "ls", "--json"]), r#"{"ok":[]}"#);
    // Card verbs other than press and focus are for peers too: posting a card passes the
    // policy (the deny table matches `card press`, not every `card` verb).
    let card = ["message", "card", "post", "{\"id\":\"c1\"}"].map(String::from);
    assert_eq!(net_policy(&card), Ok(()));
    for verb in ["get", "ls", "show", "dismiss", "spec"] {
        let req = ["message", "card", verb].map(String::from);
        assert_eq!(net_policy(&req), Ok(()), "{verb}");
    }
    // Scripts run only locally: the same request that peers are refused answers here.
    assert_eq!(
        run(vec!["script".into(), "run".into(), "Lock".into()], Flags::default()).to_line(),
        r#"{"error":"script: no command \"Lock\" (none in [[script.commands]])"}"#
    );
    // Network callers may read the toggle; it stays off by default.
    assert_eq!(
        net_reply(&["remote", "status", "--json"]),
        r#"{"ok":{"error":null,"events":false,"last":null,"listening":[],"on":false,"peers":[],"port":7419}}"#
    );
}
