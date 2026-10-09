//! Control socket replies: scripts parse them. Text answers stay `{"ok":"<text>"}` with or
//! without `--json`; a module that reads `Cx::json` and answers a JSON object or array gets
//! `{"ok":<value>}`; errors are `{"error":"<message>"}`.

use crate::config::Config;
use crate::core::control::{Reply, split_json};
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

/// The reply line for request `words`, as the control socket builds it.
fn reply(registry: &mut Registry, words: &[&str], cx: &mut Cx) -> String {
    let (words, json) = split_json(words.iter().map(|w| (*w).to_string()).collect());
    cx.json = json;
    let result = registry.command(&words, cx);
    Reply::answer(result, json).to_line()
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
