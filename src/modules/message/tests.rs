//! Module tests over one in-memory store, a fake clock and recording fakes for the panel,
//! notifications, the pasteboard and links. Nothing reaches `AppKit`.

use std::cell::RefCell;

use super::*;
use crate::config::parse;
use crate::core::Ranker;
use crate::core::store::Store;

/// 2026-10-09 18:31:01 UTC.
pub(super) const TS: i64 = 1_791_570_661;

thread_local! {
    /// What the fakes saw, in order.
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(line: String) {
    LOG.with(|l| l.borrow_mut().push(line));
}

pub(super) fn take_log() -> Vec<String> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

pub(super) fn inbox(config: &str) -> Inbox {
    let mut m = Inbox {
        env: Env {
            now: || TS,
            utc_offset: |_| -25_200,
            new_id: || "m1".into(),
            show: |id, content, p, o| {
                let Content::Text(card) = content;
                log(format!(
                    "show {id} {}|{}|{}|{}|{:?}|{}|{:?}|{}|{}|{}|{}",
                    card.header, card.time, card.context, card.body, card.link, card.pending,
                    p.corner, p.max_cards, o.timeout_secs, o.sound, o.sticky
                ));
                true
            },
            dismiss: |id| {
                log(format!("dismiss {id}"));
                true
            },
            hide: || log("hide".into()),
            notify: |id, title, body| log(format!("notify {id}|{title}|{body}")),
            copy: |text| log(format!("copy {text}")),
            open_url: |url| log(format!("open {url}")),
        },
        settings: Settings::default(),
        dismissed: HashSet::new(),
    };
    m.configure(&parse(config).unwrap().section("message").unwrap().unwrap()).unwrap();
    m
}

pub(super) struct Fixture {
    pub(super) store: Store,
    ranker: Ranker,
    /// Requests come from this Mac, not the network.
    pub(super) local: bool,
}

impl Fixture {
    pub(super) fn new() -> Fixture {
        let store = Store::in_memory();
        store.migrate("message", store::MIGRATIONS).unwrap();
        Fixture { store, ranker: Ranker::new(), local: false }
    }

    pub(super) fn cx<R>(&mut self, query: &str, json: bool, f: impl FnOnce(&mut Cx) -> R) -> R {
        f(&mut Cx {
            query,
            store: &self.store,
            ranker: &mut self.ranker,
            hide: || log("launcher hide".into()),
            json,
            remote: !self.local,
        })
    }

    pub(super) fn run(&mut self, m: &mut Inbox, json: bool, words: &[&str]) -> Result<String, String> {
        let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
        self.cx("", json, |cx| m.command(&args, cx))
    }
}

#[test]
fn a_post_shows_the_panel_and_lands_in_history() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    assert_eq!(f.run(&mut m, false, &["post", "**Done**:", "see", "PR"]), Ok("m1".into()));
    assert_eq!(take_log(), ["show m1 Messages|11:31||Done: see PR|None|false|TopRight|4|20|true|false"]);
    let json = f.run(&mut m, true, &["ls"]).unwrap();
    let list: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(list[0]["id"], "m1");
    assert_eq!(list[0]["body"], "**Done**: see PR");
    assert_eq!(f.run(&mut m, false, &["ls"]).unwrap(), "m1\t11:31\tDone: see PR");
}

#[test]
fn a_reply_replaces_its_pending_post() {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nname = \"KOTA\"\nstyle = \"both\""));
    f.run(&mut m, false, &["post", "--pending", "--id", "k1", "what's", "on", "today?"]).unwrap();
    assert_eq!(take_log(), ["show k1 KOTA|11:31||what's on today?|None|true|TopRight|4|20|false|false"]);
    let posted = f
        .run(&mut m, true, &["post", "--reply-to", "k1", "--url", "https://cal.com/x", "--id", "r1", "Two", "calls."])
        .unwrap();
    assert_eq!(posted, r#"{"id":"r1","replaced":true}"#);
    assert_eq!(
        take_log(),
        [
            "dismiss k1",
            "show r1 KOTA|11:31|Re: what's on today?|Two calls.|Some(\"https://cal.com/x\")|false|TopRight|4|20|true|false",
            "notify r1|KOTA|Two calls.",
        ]
    );
    assert_eq!(f.run(&mut m, false, &["ls"]).unwrap(), "r1\t11:31\tTwo calls.");
    // A reply to a message that is not pending keeps it and quotes it.
    f.run(&mut m, false, &["post", "--reply-to", "r1", "--title", "Update", "One", "moved."]).unwrap();
    assert!(take_log()[0].starts_with("show m1 Update|11:31|Re: Two calls.|One moved."));
    assert_eq!(f.store.messages(10).len(), 2);
}

#[test]
fn a_reply_under_its_placeholder_id_redraws_that_card() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    f.run(&mut m, false, &["post", "--pending", "--id", "k1", "q?"]).unwrap();
    f.run(&mut m, false, &["post", "--reply-to", "k1", "--id", "k1", "a."]).unwrap();
    let log = take_log();
    assert_eq!(log.len(), 2, "no dismiss: {log:?}");
    assert!(log[1].starts_with("show k1 Messages|11:31|Re: q?|a.|"), "{log:?}");
}

#[test]
fn style_and_placement_follow_config() {
    let config = "[message]\nwidth = 400\nstyle = \"notification\"\nposition = \"bottom-left\"\nmax_cards = 2\ntimeout_secs = 0\nsound = false";
    let (mut f, mut m) = (Fixture::new(), inbox(config));
    f.run(&mut m, false, &["post", "--pending", "waiting"]).unwrap();
    assert_eq!(take_log(), [""; 0], "a pending post is not a notification");
    f.run(&mut m, false, &["post", "--id", "n2", "hi"]).unwrap();
    assert_eq!(take_log(), ["notify n2|Messages|hi"]);
    // `show` always uses the panel.
    assert_eq!(f.run(&mut m, false, &["show", "n2"]), Ok("Showing n2".into()));
    assert_eq!(take_log(), ["show n2 Messages|11:31||hi|None|false|BottomLeft|2|0|false|false"]);
    let none = inbox("[message]\nstyle = \"none\"");
    f.cx("", false, |cx| none.post(&["quiet".to_string()], cx)).unwrap();
    assert_eq!(take_log(), [""; 0]);
}

#[test]
fn verbs_report_errors() {
    let (mut f, mut m) = (Fixture::new(), inbox(""));
    assert_eq!(f.run(&mut m, false, &["show"]), Err("No messages".into()));
    assert_eq!(f.run(&mut m, false, &["show", "nope"]), Err("No message nope".into()));
    assert_eq!(f.run(&mut m, false, &["hide"]), Ok("Hidden".into()));
    assert_eq!(take_log(), ["hide"]);
    assert!(f.run(&mut m, false, &["post"]).unwrap_err().starts_with("usage"));
    assert!(f.run(&mut m, false, &["ls", "--limit", "x"]).unwrap_err().contains("not a number"));
    assert!(f.run(&mut m, false, &["ls", "x"]).unwrap_err().starts_with("usage"));
    assert!(f.run(&mut m, false, &["nope"]).is_err());
    f.run(&mut m, false, &["post", "a"]).unwrap();
    take_log();
    assert_eq!(f.run(&mut m, false, &["ls", "--limit", "0"]), Ok(String::new()));
    assert_eq!(f.run(&mut m, false, &["show"]), Ok("Showing m1".into()));
}

#[test]
fn history_is_capped() {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nmax_history = 2"));
    for id in ["a", "b", "c"] {
        f.run(&mut m, false, &["post", "--id", id, id]).unwrap();
    }
    let ids: Vec<_> = f.store.messages(10).into_iter().map(|m| m.id).collect();
    assert_eq!(ids, ["c", "b"]);
}

#[test]
fn bad_tables_are_refused() {
    let bad = |text: &str| {
        let mut m = Inbox::default();
        m.configure(&parse(text).unwrap().section("message").unwrap().unwrap()).unwrap_err()
    };
    assert!(bad("[message]\nname = \" \"").contains("name"));
    assert!(bad("[message]\nwidth = 100.0").contains("width"));
    assert!(bad("[message]\nmax_history = 0").contains("max_history"));
    assert!(bad("[message]\nmax_cards = 0").contains("max_cards"));
    assert!(bad("[message]\nstyle = \"toast\"").starts_with("[message]: "));
    assert!(bad("[message]\nposition = \"middle\"").starts_with("[message]: "));
    assert!(bad("[message]\ncolour = 1").starts_with("[message]: "));
}

/// An outcome as a short string, so a test compares it with `assert_eq!`.
fn kind(o: Outcome) -> String {
    match o {
        Outcome::Hide => "hide".into(),
        Outcome::Stay(s) => format!("stay {s:?}"),
        Outcome::Push(v) => format!("push {}/{}", v.module, v.name),
        other => format!("{other:?}"),
    }
}

/// A KOTA inbox with a pending post `p` and a linked post `a`, and its `recent` view.
fn seeded() -> (Fixture, Inbox, ListView) {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nname = \"KOTA\"\nhotkey = \"cmd+alt+M\""));
    let root = f.cx("", false, |cx| m.items(cx));
    assert_eq!((root[0].title.as_str(), root[0].subtitle.as_str()), ("KOTA", "No messages yet"));
    f.run(&mut m, false, &["post", "--id", "p", "--pending", "q?"]).unwrap();
    f.run(&mut m, false, &["post", "--id", "a", "--title", "PR", "--url", "https://x.y", "- one\n- two"]).unwrap();
    take_log();
    let mut view = f.cx("", false, |cx| m.open(RECENT, cx)).unwrap();
    f.cx("", false, |cx| m.refresh(&mut view, cx));
    (f, m, view)
}

#[test]
fn the_launcher_lists_messages() {
    let (mut f, mut m, mut view) = seeded();
    let root = f.cx("", false, |cx| m.items(cx));
    assert_eq!(root[0].subtitle, "• one • two");
    assert_eq!(kind(f.cx("", false, |cx| m.activate(&root[0].id, cx))), "push message/recent");
    assert_eq!(f.cx("", false, |cx| m.actions(&root[0].id, cx)), []);
    assert_eq!(view.footer, "KOTA  ·  ↵ copies  ·  ⌘K show or open");
    assert!(f.cx("", false, |cx| m.open("nope", cx)).is_none());
    let rows: Vec<_> = view.items.iter().map(|i| (i.title.as_str(), i.subtitle.as_str(), i.accessory.as_str())).collect();
    assert_eq!(rows, [("PR: • one • two", "11:31", "Link"), ("q?", "11:31", "Pending")]);
    f.cx("q?", false, |cx| m.refresh(&mut view, cx));
    assert_eq!(view.items[0].title, "q?");

    assert_eq!(m.hotkeys(), [Binding { spec: "cmd+alt+M".into(), key: Ok(RECENT.into()) }]);
    assert!(f.cx("", false, |cx| m.hotkey(RECENT, cx)).is_some());
    assert!(f.cx("", false, |cx| m.hotkey("x", cx)).is_none());
    assert_eq!(inbox("").hotkeys(), []);
    assert!(m.verbs().starts_with("message post"));
    assert_eq!(m.migrations(), store::MIGRATIONS);
}

#[test]
fn rows_copy_reshow_and_open() {
    let (mut f, mut m, view) = seeded();
    let link = view.items[0].id.clone();
    assert_eq!(kind(f.cx("", false, |cx| m.activate(&link, cx))), "stay Some(\"Copied message\")");
    assert_eq!(take_log(), ["copy • one\n• two"]);
    let keys = |m: &mut Inbox, f: &mut Fixture, id| f.cx("", false, |cx| m.actions(id, cx)).iter().map(|a| a.key).collect::<Vec<_>>();
    assert_eq!(keys(&mut m, &mut f, &link), ["show", "open", "copy"]);
    assert_eq!(keys(&mut m, &mut f, &view.items[1].id), ["show", "copy"]);
    assert_eq!(kind(f.cx("", false, |cx| m.act(&link, "open", cx))), "hide");
    assert_eq!(kind(f.cx("", false, |cx| m.act(&link, "show", cx))), "hide");
    assert_eq!(kind(f.cx("", false, |cx| m.act(&link, "copy", cx))), "stay Some(\"Copied message\")");
    let log = take_log();
    assert_eq!(log[..2], ["launcher hide", "open https://x.y"]);
    assert!(log[3].starts_with("show a PR|"), "{log:?}");
    assert_eq!(kind(f.cx("", false, |cx| m.act(&view.items[1].id, "open", cx))), "stay None");
    let gone = ItemId::new("message", "gone");
    assert_eq!(kind(f.cx("", false, |cx| m.act(&gone, "show", cx))), "stay None");
    assert_eq!(kind(f.cx("", false, |cx| m.activate(&gone, cx))), "stay None");
}

#[test]
fn the_config_example_documents_every_setting() {
    crate::config::example::assert_documents::<Settings>("message");
}
