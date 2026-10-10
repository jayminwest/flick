//! Chat threads in the store (plan pl-75d3, flick-a7b0): `post --thread` and `--partial`,
//! card threads, per-scope history and the `thread`/`threads` verbs.

use super::store::{Messages, Progress};
use super::tests::{Fixture, inbox, take_log};

#[test]
fn a_streamed_reply_updates_one_message_quietly_until_done() {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nstyle = \"both\""));
    let post = |f: &mut Fixture, m: &mut _, w: &[&str]| f.run(m, true, &[&["post"], w].concat()).unwrap();
    assert_eq!(post(&mut f, &mut m, &["--thread", "t1", "--id", "r1", "--partial", "Thinking"]), r#"{"id":"r1","replaced":false}"#);
    post(&mut f, &mut m, &["--thread", "t1", "--id", "r1", "--partial", "Thinking", "more"]);
    // No sound and no notification while it streams; the card redraws in place by id.
    assert_eq!(
        take_log(),
        [
            "show r1 Messages|11:31||Thinking|None|false|TopRight|4|20|false|false",
            "show r1 Messages|11:31||Thinking more|None|false|TopRight|4|20|false|false",
        ]
    );
    assert_eq!(f.run(&mut m, false, &["ls"]).unwrap(), "r1\t11:31\tThinking more (partial)");
    // The final post drops --partial (and may drop --thread): sound and notification.
    post(&mut f, &mut m, &["--id", "r1", "Done."]);
    assert_eq!(
        take_log(),
        ["show r1 Messages|11:31||Done.|None|false|TopRight|4|20|true|false", "notify r1|Messages|Done."]
    );
    let r1 = f.store.message("r1").unwrap();
    assert_eq!((r1.thread.as_deref(), r1.state, r1.body.as_str()), (Some("t1"), Progress::Done, "Done."));
    assert_eq!(f.store.messages(10).len(), 1);
}

#[test]
fn thread_verbs_read_the_store() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    assert_eq!(f.run(&mut m, false, &["threads"]), Ok(String::new()));
    assert_eq!(f.run(&mut m, true, &["threads"]), Ok("[]".into()));
    assert_eq!(f.run(&mut m, false, &["thread", "t1"]), Err("No thread t1".into()));
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "q", "**Plan** for today?"]).unwrap();
    f.run(&mut m, false, &["post", "--thread", "t1", "--reply-to", "q", "--id", "a", "Two calls."]).unwrap();
    f.run(&mut m, false, &["post", "--thread", "t2", "--id", "b", "--partial", "Hm"]).unwrap();
    let card = r#"{"id":"c","title":"Ship?","thread":"t1"}"#;
    f.run(&mut m, false, &["card", "post", card]).unwrap();
    f.run(&mut m, false, &["post", "loose"]).unwrap();
    take_log();
    assert_eq!(
        f.run(&mut m, false, &["threads"]).unwrap(),
        "t1\t11:31\t3 messages\tPlan for today?\nt2\t11:31\t1 message\tHm"
    );
    assert_eq!(f.run(&mut m, false, &["threads", "--limit", "1"]).unwrap().lines().count(), 1);
    let json: serde_json::Value = serde_json::from_str(&f.run(&mut m, true, &["threads"]).unwrap()).unwrap();
    assert_eq!(json[0]["id"], "t1");
    assert_eq!(json[0]["messages"], 3);
    assert_eq!(
        f.run(&mut m, false, &["thread", "t1"]).unwrap(),
        "q\t11:31\tPlan for today?\na\t11:31\tTwo calls.\nc\t11:31\tShip?"
    );
    assert_eq!(f.run(&mut m, false, &["thread", "t1", "--limit", "1"]).unwrap(), "c\t11:31\tShip?");
    assert_eq!(f.run(&mut m, false, &["thread", "t2"]).unwrap(), "b\t11:31\tHm (partial)");
    let json: serde_json::Value = serde_json::from_str(&f.run(&mut m, true, &["thread", "t1"]).unwrap()).unwrap();
    assert_eq!((json[1]["id"].as_str(), json[1]["context"].as_str()), (Some("a"), Some("**Plan** for today?")));
    assert_eq!(json[2]["card"]["thread"], "t1");
}

#[test]
fn thread_verbs_refuse_bad_input() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    f.run(&mut m, false, &["post", "--thread", "t1", "x"]).unwrap();
    take_log();
    assert_eq!(f.run(&mut m, false, &["thread", "t1", "--limit", "0"]), Ok(String::new()));
    let usage = "usage: flick message threads [--limit n] | thread <t> [--limit n]";
    assert_eq!(f.run(&mut m, false, &["thread"]), Err(usage.into()));
    assert_eq!(f.run(&mut m, false, &["threads", "x"]), Err(usage.into()));
    assert_eq!(f.run(&mut m, false, &["threads", "--limit", "x"]), Err("--limit x: not a number".into()));
    assert!(f.run(&mut m, false, &["thread", "a b"]).unwrap_err().contains("a thread id is"));
    assert!(f.run(&mut m, false, &["post", "--thread", "a b", "x"]).unwrap_err().contains("an id is"));
}

#[test]
fn chat_history_never_evicts_unthreaded_history() {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nmax_history = 1\nchat_history = 2\nchat_threads = 1"));
    f.run(&mut m, false, &["post", "--id", "h", "keep me"]).unwrap();
    for (id, t) in [("a", "t1"), ("b", "t1"), ("c", "t1"), ("d", "t2")] {
        f.run(&mut m, false, &["post", "--thread", t, "--id", id, id]).unwrap();
    }
    take_log();
    let ids: Vec<_> = f.store.messages(10).into_iter().map(|m| m.id).collect();
    assert_eq!(ids, ["d", "h"], "t1 went with chat_threads = 1; h stays");
    assert!(f.run(&mut m, false, &["thread", "t1"]).is_err());
}

#[test]
fn chat_limits_must_be_positive() {
    for key in ["chat_history", "chat_threads"] {
        let mut m = super::Inbox::default();
        let table = crate::config::parse(&format!("[message]\n{key} = 0")).unwrap();
        let err = crate::core::Module::configure(&mut m, &table.section("message").unwrap().unwrap()).unwrap_err();
        assert_eq!(err, format!("[message]: {key} must be at least 1"));
    }
}
