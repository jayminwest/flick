//! The remote-use grant: agent sessions (`cx.remote`) read activity only while the user
//! allows it, never change the grant or recording, and see no titles by default.

use super::*;
use crate::modules::activity::remote::REFUSED;

const AGENT: &str = "activity: not allowed from an agent session (--remote)";

/// Run `words` as a remote caller (`--remote`, with `--json` when `json`).
fn remote(a: &mut Activity, cx: &mut Cx, words: &str, json: bool) -> Result<String, String> {
    (cx.remote, cx.json) = (true, json);
    let reply = run(a, cx, words);
    (cx.remote, cx.json) = (false, false);
    reply
}

#[test]
fn remote_reads_need_a_live_grant() {
    with_cx(|cx| {
        let mut a = activity("");
        for words in ["today", "week", "today --by task", "spans", "spans --since week"] {
            assert_eq!(remote(&mut a, cx, words, true), Err(REFUSED.into()), "{words}");
            assert!(run(&mut a, cx, words).is_ok(), "{words}");
        }
        // Status stays allowed.
        assert!(remote(&mut a, cx, "status", false).unwrap().starts_with("recording: off"));
        assert_eq!(
            remote(&mut a, cx, "remote status", false).unwrap(),
            "Agents may not read activity\nlast read: never"
        );
        assert_eq!(cx.store.remote_last(), None);
        assert_eq!(run(&mut a, cx, "remote allow").unwrap(), "Agents may read activity · 60 min left");
        at(T + 10);
        assert!(remote(&mut a, cx, "today", true).unwrap().starts_with("{\"range\":\"today\""));
        assert_eq!(cx.store.remote_last(), Some(T + 10));
        // A refused read (bad arguments) is not a read.
        at(T + 20);
        assert!(remote(&mut a, cx, "spans --since never", false).is_err());
        assert_eq!(cx.store.remote_last(), Some(T + 10));
        assert_eq!(
            remote(&mut a, cx, "remote status", true).unwrap(),
            format!(r#"{{"granted":true,"until":{},"last_read":{}}}"#, T + 3600, T + 10)
        );
    });
}

#[test]
fn agents_never_grant_record_or_forget() {
    with_cx(|cx| {
        let mut a = activity("");
        for words in ["remote allow", "remote allow always", "remote deny", "remote x", "on", "off", "forget all --yes"] {
            assert_eq!(remote(&mut a, cx, words, false), Err(AGENT.into()), "{words}");
        }
        assert!(remote(&mut a, cx, "remote", false).unwrap_err().contains("usage"));
        assert!(remote(&mut a, cx, "nope", false).is_err());
        assert_eq!((cx.store.remote_until(), cx.store.recording()), (0, false));
        // A live grant changes none of that.
        run(&mut a, cx, "remote allow always").unwrap();
        assert_eq!(remote(&mut a, cx, "remote deny", false), Err(AGENT.into()));
        assert_eq!(cx.store.remote_until(), i64::MAX);
    });
}

/// The `remote:` line of `activity status`.
fn status_remote(a: &mut Activity, cx: &mut Cx) -> String {
    let status = run(a, cx, "status").unwrap();
    status.lines().find_map(|l| l.strip_prefix("remote: ")).unwrap_or_default().to_owned()
}

#[test]
fn status_shows_the_grant() {
    with_cx(|cx| {
        let mut a = activity("");
        assert_eq!(status_remote(&mut a, cx), "off");
        run(&mut a, cx, "remote allow 5").unwrap();
        at(T + 60);
        assert_eq!(status_remote(&mut a, cx), "on · 4 min left");
        assert!(remote(&mut a, cx, "status", false).unwrap().contains("\nremote: on · 4 min left\n"));
        at(T + 300);
        assert_eq!(status_remote(&mut a, cx), "off", "an expired grant is off");
        run(&mut a, cx, "remote allow always").unwrap();
        assert_eq!(status_remote(&mut a, cx), "on · until revoked");
        run(&mut a, cx, "remote deny").unwrap();
        assert_eq!(status_remote(&mut a, cx), "off");
    });
}

#[test]
fn the_grant_expires_lazily() {
    with_cx(|cx| {
        let mut a = activity("");
        for bad in ["remote allow 0", "remote allow -5", "remote allow x", "remote allow 1 2", "remote deny x"] {
            assert!(run(&mut a, cx, bad).unwrap_err().contains("usage"), "{bad}");
        }
        assert_eq!(run(&mut a, cx, "remote allow 1").unwrap(), "Agents may read activity · 1 min left");
        at(T + 59);
        assert!(remote(&mut a, cx, "today", false).is_ok());
        assert!(run(&mut a, cx, "remote status").unwrap().starts_with("Agents may read activity · <1 min left"));
        at(T + 60);
        assert_eq!(remote(&mut a, cx, "today", false), Err(REFUSED.into()));
        assert_eq!(
            remote(&mut a, cx, "remote status", true).unwrap(),
            format!(r#"{{"granted":false,"until":null,"last_read":{}}}"#, T + 59)
        );
        assert_eq!(run(&mut a, cx, "remote allow always").unwrap(), "Agents may read activity · until revoked");
        at(i64::MAX - 1);
        cx.json = true;
        assert!(run(&mut a, cx, "remote status").unwrap().starts_with(r#"{"granted":true,"until":null"#));
        cx.json = false;
        assert_eq!(run(&mut a, cx, "remote deny").unwrap(), "Agents may not read activity");
        assert_eq!(remote(&mut a, cx, "week", false), Err(REFUSED.into()));
    });
}

/// `today --json` top titles and `spans --json` titles, as a remote caller.
fn remote_titles(a: &mut Activity, cx: &mut Cx) -> (serde_json::Value, serde_json::Value) {
    let today: serde_json::Value = serde_json::from_str(&remote(a, cx, "today", true).unwrap()).unwrap();
    let spans: serde_json::Value = serde_json::from_str(&remote(a, cx, "spans", true).unwrap()).unwrap();
    (today["top_titles"].clone(), spans[0]["title"].clone())
}

#[test]
fn remote_replies_drop_titles_unless_configured() {
    with_cx(|cx| {
        for (table, shown) in [("titles = true", false), ("titles = true\nremote_titles = true", true)] {
            let mut a = activity(&format!("[activity]\n{table}"));
            title(Some("secret.rs"));
            run(&mut a, cx, "on").unwrap();
            run(&mut a, cx, "remote allow").unwrap();
            at(T + 30);
            let (top, span) = remote_titles(&mut a, cx);
            assert_eq!(top.as_array().unwrap().is_empty(), !shown, "{table}");
            assert_eq!(span == "secret.rs", shown, "{table}");
            assert!(!remote(&mut a, cx, "spans", false).unwrap().contains("secret") || shown);
            assert!(!remote(&mut a, cx, "today", false).unwrap().contains("secret") || shown);
            // Local callers always see them.
            assert!(run(&mut a, cx, "spans").unwrap().ends_with("secret.rs"));
            run(&mut a, cx, "forget all --yes").unwrap();
            at(T);
        }
    });
}

#[test]
fn the_root_item_allows_an_hour_or_revokes() {
    with_cx(|cx| {
        let mut a = activity("");
        let item = |a: &mut Activity, cx: &mut Cx| {
            a.items(cx).into_iter().find(|i| i.id.as_str() == "activity:remote").unwrap().title
        };
        assert_eq!(item(&mut a, cx), "Allow Agents to Read Activity (1 h)");
        let id = ItemId::new("activity", "remote");
        let Outcome::Stay(Some(msg)) = a.activate(&id, cx) else { unreachable!() };
        assert_eq!(msg, "Agents may read activity · 60 min left");
        at(T + 18 * 60 + 5);
        assert_eq!(item(&mut a, cx), "Revoke Agent Activity Access · 41 min left");
        let Outcome::Stay(Some(msg)) = a.activate(&id, cx) else { unreachable!() };
        assert_eq!(msg, "Agents may not read activity");
        assert_eq!(item(&mut a, cx), "Allow Agents to Read Activity (1 h)");
        // An expired grant reads as off.
        run(&mut a, cx, "remote allow 1").unwrap();
        at(T + 18 * 60 + 65);
        assert_eq!(item(&mut a, cx), "Allow Agents to Read Activity (1 h)");
    });
}
