//! Phone app opens (flick-b418): strict parsing, idempotent adds, no grant needed to add,
//! and reports that list opens next to Mac spans under the read grant.

use super::*;
use crate::modules::activity::phone::{PhoneOpen, parse};
use crate::modules::activity::remote::REFUSED;
use crate::modules::activity::store::PhoneOpens;

fn add(id: &str, app: &str, at: i64) -> String {
    format!("phone add --id {id} --device jaymins-iphone --app {app} --at {at}")
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

/// The error `parse` gives for `words`, with the clock at `T`.
fn refusal(words: &[&str]) -> String {
    parse(&args(words), T).unwrap_err()
}

#[test]
fn adds_are_idempotent_on_the_event_id() {
    with_cx(|cx| {
        let mut a = activity("");
        let first = add("E1", "com.apple.mobilesafari", T - 60);
        assert_eq!(run(&mut a, cx, &first).unwrap(), "Stored phone open E1 from jaymins-iphone");
        assert_eq!(run(&mut a, cx, &first).unwrap(), "Already stored phone open E1 from jaymins-iphone");
        cx.json = true;
        let json = run(&mut a, cx, &first).unwrap();
        assert_eq!(json, r#"{"id":"E1","device":"jaymins-iphone","stored":false}"#);
        let second = run(&mut a, cx, &add("E2", "Instagram", T)).unwrap();
        assert_eq!(second, r#"{"id":"E2","device":"jaymins-iphone","stored":true}"#);
        cx.json = false;
        // The same id with other fields is the phone's bug: refused, the first kept.
        assert_eq!(
            run(&mut a, cx, &add("E1", "Instagram", T - 60)).unwrap_err(),
            "activity phone: event E1 from jaymins-iphone is already stored with other fields"
        );
        // Ids are per device.
        let other = "phone add --id E1 --device ipad --app Instagram --at 999000";
        assert!(run(&mut a, cx, other).unwrap().starts_with("Stored"));
        let opens = cx.store.phone_opens(0);
        let apps: Vec<_> = opens.iter().map(|o| (o.device.as_str(), o.id.as_str(), o.app.as_str())).collect();
        assert_eq!(
            apps,
            [("ipad", "E1", "Instagram"), ("jaymins-iphone", "E1", "com.apple.mobilesafari"), ("jaymins-iphone", "E2", "Instagram")]
        );
    });
}

#[test]
fn reason_and_minutes_are_stored() {
    with_cx(|cx| {
        let mut a = activity("");
        let words = args(&[
            "phone", "add", "--minutes", "15", "--reason", "reply to Sam", "--app", "Messages", "--at", "999990",
            "--device", "jaymins-iphone", "--id", "8C1F-AA",
        ]);
        assert!(a.command(&words, cx).unwrap().starts_with("Stored"));
        assert_eq!(a.command(&words, cx).unwrap(), "Already stored phone open 8C1F-AA from jaymins-iphone");
        let open = PhoneOpen {
            device: "jaymins-iphone".into(),
            id: "8C1F-AA".into(),
            app: "Messages".into(),
            at: 999_990,
            reason: Some("reply to Sam".into()),
            minutes: Some(15),
        };
        assert_eq!(cx.store.phone_opens(0), [open]);
    });
}

#[test]
fn every_field_is_checked() {
    // A real clock: `T` is less than 30 days after the epoch.
    const NOW: i64 = 1_760_000_000;
    let ok = ["--id", "e1", "--device", "d", "--app", "Safari", "--at", "1760000000"];
    assert_eq!(parse(&args(&ok), NOW).unwrap().at, NOW);
    let with = |flag: &str, value: &str| {
        let mut w = args(&ok);
        let i = w.iter().position(|x| x == flag).unwrap();
        w[i + 1] = value.into();
        parse(&w, NOW)
    };
    let long_id = "a".repeat(65);
    let long_app = "a".repeat(129);
    for (flag, value, why) in [
        ("--id", "", "needs 1 to 64 characters"),
        ("--id", long_id.as_str(), "needs 1 to 64 characters"),
        ("--id", "a b", "only letters, digits and . _ - are allowed"),
        ("--id", "é", "only letters, digits and . _ - are allowed"),
        ("--device", "a:b", "only letters, digits and . _ - are allowed"),
        ("--app", "", "needs 1 to 128 characters"),
        ("--app", long_app.as_str(), "needs 1 to 128 characters"),
        ("--app", "a\tb", "no control characters or space at either end"),
        ("--app", " Safari", "no control characters or space at either end"),
        ("--at", "-5", "digits only (unix seconds or minutes)"),
        ("--at", "1e6", "digits only (unix seconds or minutes)"),
        ("--at", "1000000000000", "digits only (unix seconds or minutes)"),
        ("--at", "1760000301", "more than 30 days ago or ahead of this Mac's clock"),
        ("--at", "1757407999", "more than 30 days ago or ahead of this Mac's clock"),
    ] {
        assert_eq!(with(flag, value).unwrap_err(), format!("activity phone: bad {flag}: {why}"), "{flag} {value:?}");
    }
    assert!(with("--at", "1760000300").is_ok() && with("--at", "1757408000").is_ok());
    assert!(with("--id", "A1:b_2.c-3").is_ok() && with("--app", "Threads, an Instagram app").is_ok());
    let more = |extra: &[&str]| parse(&[args(&ok), args(extra)].concat(), NOW);
    assert_eq!(more(&["--minutes", "0"]).unwrap_err(), "activity phone: bad --minutes: must be 1 to 1440");
    assert_eq!(more(&["--minutes", "1441"]).unwrap_err(), "activity phone: bad --minutes: must be 1 to 1440");
    assert_eq!(more(&["--reason", "a\nb"]).unwrap_err(), "activity phone: bad --reason: no control characters or space at either end");
    assert_eq!(more(&["--reason", &"r".repeat(281)]).unwrap_err(), "activity phone: bad --reason: needs 1 to 280 characters");
    assert_eq!(more(&["--minutes", "1440"]).unwrap().minutes, Some(1440));
}

#[test]
fn flags_are_each_given_once() {
    assert_eq!(refusal(&["--id", "a", "--id", "b"]), "activity phone: bad --id: given twice");
    assert_eq!(refusal(&["--id"]), "activity phone: bad --id: needs a value");
    assert_eq!(refusal(&["--id", "a"]), "activity phone: bad --device: missing");
    assert!(refusal(&["--title", "x"]).starts_with("activity phone: unknown argument \"--title\"; activity phone: usage:"));
    assert!(refusal(&["E1"]).starts_with("activity phone: unknown argument \"E1\""));
    with_cx(|cx| {
        let mut a = activity("");
        for words in ["phone", "phone ls", "phone add"] {
            let e = run(&mut a, cx, words).unwrap_err();
            assert!(e.starts_with("activity phone: "), "{words}: {e}");
        }
    });
}

#[test]
fn a_failed_write_is_not_a_bad_event() {
    // Errors that are not `activity phone: ...` tell the phone to retry later.
    at(T);
    crate::core::test_cx("", |cx| {
        let a = activity("");
        let e = a.phone(&args(&["add", "--id", "e", "--device", "d", "--app", "A", "--at", "1000000"]), cx).unwrap_err();
        assert!(e.starts_with("activity: could not store the phone open: "), "{e}");
    });
}

/// Run `words` as a remote caller.
fn remote(a: &mut Activity, cx: &mut Cx, words: &str, json: bool) -> Result<String, String> {
    (cx.remote, cx.json) = (true, json);
    let reply = run(a, cx, words);
    (cx.remote, cx.json) = (false, false);
    reply
}

#[test]
fn remote_callers_add_without_the_grant_and_read_with_it() {
    with_cx(|cx| {
        let mut a = activity("[[activity.rules]]\napp = \"instagram\"\ncategory = \"social\"");
        assert!(remote(&mut a, cx, &add("E1", "Instagram", T - 30), false).unwrap().starts_with("Stored"));
        assert_eq!(cx.store.remote_last(), None, "an add is not a read");
        assert_eq!(remote(&mut a, cx, "spans", true), Err(REFUSED.into()));
        run(&mut a, cx, "on").unwrap();
        at(T + 60);
        run(&mut a, cx, "remote allow").unwrap();
        let spans: serde_json::Value = serde_json::from_str(&remote(&mut a, cx, "spans", true).unwrap()).unwrap();
        let open = &spans[0];
        assert_eq!((open["source"].clone(), open["device"].clone()), ("phone".into(), "jaymins-iphone".into()));
        assert_eq!((open["start"].clone(), open["end"].clone(), open["secs"].clone()), ((T - 30).into(), (T - 30).into(), 0.into()));
        assert_eq!((open["app"].clone(), open["category"].clone()), ("Instagram".into(), "social".into()));
        assert_eq!((open["reason"].clone(), open["minutes"].clone()), (serde_json::Value::Null, serde_json::Value::Null));
        let mac = &spans[1];
        assert_eq!((mac["source"].clone(), mac["device"].clone(), mac["name"].clone()), ("mac".into(), serde_json::Value::Null, "Safari".into()));
        let today: serde_json::Value = serde_json::from_str(&remote(&mut a, cx, "today", true).unwrap()).unwrap();
        assert_eq!(today["phone"], serde_json::json!({"opens": 1, "by_app": [{"name": "Instagram", "opens": 1}]}));
        assert_eq!(today["recorded_secs"], 60, "opens add no recorded time");
    });
}

#[test]
fn text_reports_list_phone_opens() {
    with_cx(|cx| {
        let mut a = activity("");
        for (id, app) in [("a", "Instagram"), ("b", "Safari"), ("c", "Instagram")] {
            run(&mut a, cx, &add(id, app, T - 100)).unwrap();
        }
        run(&mut a, cx, "phone add --id old --device jaymins-iphone --app Safari --at 900000").unwrap();
        let today = run(&mut a, cx, "today").unwrap();
        assert!(today.ends_with("\nPhone opens (3)\n         2  Instagram\n         1  Safari"), "{today}");
        let spans = run(&mut a, cx, "spans").unwrap();
        assert_eq!(spans.lines().next(), Some("1970-01-12 13:45\topen\tInstagram\tInstagram\tjaymins-iphone"));
        assert_eq!(spans.lines().count(), 3, "{spans}");
        assert_eq!(run(&mut a, cx, "spans --since 1970-01-11").unwrap().lines().count(), 4);
        let empty = activity("");
        assert_eq!(phone_text(&empty, cx), "");
    });
}

fn phone_text(a: &Activity, cx: &mut Cx) -> String {
    cx.store.phone_forget_since(i64::MIN);
    a.report("today", cx.store).phone.text()
}

#[test]
fn forget_deletes_phone_opens() {
    with_cx(|cx| {
        let mut a = activity("");
        run(&mut a, cx, &add("a", "Instagram", T - 10)).unwrap();
        run(&mut a, cx, &add("b", "Safari", T - 10)).unwrap();
        run(&mut a, cx, "phone add --id c --device jaymins-iphone --app Safari --at 900000").unwrap();
        assert_eq!(run(&mut a, cx, "forget today").unwrap_err(), "activity: forget deletes spans; add --yes");
        assert_eq!(cx.store.phone_opens(0).len(), 3);
        assert_eq!(run(&mut a, cx, "forget app Instagram --yes").unwrap(), "Deleted 0 spans and 1 phone open");
        assert_eq!(run(&mut a, cx, "forget today --yes").unwrap(), "Deleted 0 spans and 1 phone open");
        run(&mut a, cx, &add("d", "Safari", T - 10)).unwrap();
        assert_eq!(run(&mut a, cx, "forget all --yes").unwrap(), "Deleted 0 spans and 2 phone opens");
        assert!(cx.store.phone_opens(i64::MIN).is_empty());
    });
}
