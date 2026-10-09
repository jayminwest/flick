//! Front tab URLs (`urls = true`): asked for on the title triggers, answered off the main
//! thread (here: by hand, never with a real Apple Event), split spans like titles, and stay
//! out of remote replies unless `remote_urls = true`.

use super::*;
use crate::modules::activity::urls::Answer;
use crate::platform::browser::TabUrl;

const BRAVE: i32 = 6;
const ON: &str = "[activity]\nurls = true";

/// Answer the `nth` pending ask (counting from the newest, 0) with `url`, drop the others,
/// and deliver `ModuleChanged` at time `t`.
fn answer_nth(a: &mut Activity, cx: &mut Cx, t: i64, nth: usize, url: TabUrl) {
    let mut asks = ASKS.with(RefCell::take);
    let ask = asks.remove(asks.len() - 1 - nth);
    let reply = Answer { seq: ask.seq, pid: ask.pid, name: ask.name, url };
    ask.inbox.lock().unwrap().push(reply);
    at(t);
    a.on_event(Event::ModuleChanged { module: "activity" }, cx);
}

fn answer(a: &mut Activity, cx: &mut Cx, t: i64, url: &str) {
    answer_nth(a, cx, t, 0, TabUrl::Url(url.into()));
}

/// Stored spans as (start, end, app name, url).
fn urled(cx: &Cx) -> Vec<(i64, i64, String, Option<String>)> {
    let spans = cx.store.spans(0, i64::MAX).into_iter();
    spans.map(|s| (s.start, s.end, s.subject.name, s.subject.url)).collect()
}

fn brave(start: i64, end: i64, url: Option<&str>) -> (i64, i64, String, Option<String>) {
    (start, end, "Brave Browser".into(), url.map(String::from))
}

#[test]
fn urls_off_never_asks() {
    with_cx(|cx| {
        FRONT.with(|f| f.set(Some(BRAVE)));
        let mut a = activity("");
        run(&mut a, cx, "on").unwrap();
        at(T + 10);
        a.on_event(Event::WindowChanged { pid: BRAVE }, cx);
        assert_eq!(calls(), ["indicator true", "on_quit"]);
        assert_eq!(urled(cx), [brave(T, T + 10, None)]);
        assert!(run(&mut a, cx, "status").unwrap().contains("\nurls: off\n"));
    });
}

#[test]
fn an_answer_names_the_span_and_a_new_url_splits_it() {
    with_cx(|cx| {
        FRONT.with(|f| f.set(Some(BRAVE)));
        let mut a = activity(ON);
        run(&mut a, cx, "on").unwrap();
        // Titles stay off, but the window is followed for tab changes.
        assert_eq!(calls(), ["ask_url 6 com.brave.Browser", "indicator true", "on_quit", "follow 6"]);
        // Within merge_secs the answer replaces the span opened without a URL.
        answer(&mut a, cx, T + 1, "https://github.com/a");
        assert_eq!(urled(cx), [brave(T, T + 1, Some("https://github.com/a"))]);
        assert!(calls().is_empty(), "applying an answer asks nothing");
        at(T + 60);
        a.on_event(Event::WindowChanged { pid: BRAVE }, cx);
        assert_eq!(calls(), ["ask_url 6 com.brave.Browser"]);
        answer(&mut a, cx, T + 61, "https://docs.rs/");
        // The same URL again only extends.
        a.on_event(Event::WindowChanged { pid: BRAVE }, cx);
        answer(&mut a, cx, T + 70, "https://docs.rs/");
        assert_eq!(
            urled(cx),
            [brave(T, T + 61, Some("https://github.com/a")), brave(T + 61, T + 70, Some("https://docs.rs/"))]
        );
        // Back to Brave later: the span starts with the last URL read from it.
        at(T + 80);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        at(T + 90);
        a.on_event(Event::AppActivated { pid: BRAVE }, cx);
        assert_eq!(urled(cx)[3], brave(T + 90, T + 90, Some("https://docs.rs/")));
        let r: serde_json::Value = serde_json::from_str(&json(&mut a, cx, "today")).unwrap();
        assert_eq!(r["top_domains"][0]["name"], "github.com");
        assert_eq!(r["top_domains"][1]["name"], "docs.rs");
    });
}

#[test]
fn the_today_view_lists_browser_time_by_domain() {
    with_cx(|cx| {
        FRONT.with(|f| f.set(Some(BRAVE)));
        let mut a = activity(ON);
        run(&mut a, cx, "on").unwrap();
        answer(&mut a, cx, T + 1, "https://www.github.com/a");
        at(T + 30);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        at(T + 40);
        let mut view = a.open("today", cx).unwrap();
        a.refresh(&mut view, cx);
        let rows = view.items.iter().map(|i| (i.id.to_string(), i.subtitle.as_str())).collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                ("activity:row/app/Brave Browser".into(), "App"),
                ("activity:row/app/Mail".into(), "App"),
                ("activity:row/domain/github.com".into(), "Domain"),
            ]
        );
        assert_eq!(view.items[2].accessory, "30s  ·  75%");
        assert_eq!(view.footer, "Activity Today  ·  esc to go back");
    });
}

fn json(a: &mut Activity, cx: &mut Cx, words: &str) -> String {
    cx.json = true;
    let reply = run(a, cx, words).unwrap();
    cx.json = false;
    reply
}

#[test]
fn only_supported_browsers_that_are_not_excluded_are_asked() {
    with_cx(|cx| {
        let mut a = activity("[activity]\nurls = true\nexclude = [\"Google Chrome\"]");
        run(&mut a, cx, "on").unwrap(); // Safari
        at(T + 10);
        a.on_event(Event::AppActivated { pid: 7 }, cx); // Chrome, excluded
        at(T + 20);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        assert!(!calls().iter().any(|c| c.starts_with("ask_url")));
        assert!(urled(cx).iter().all(|s| s.3.is_none()));
    });
}

#[test]
fn stale_late_and_private_answers() {
    with_cx(|cx| {
        FRONT.with(|f| f.set(Some(BRAVE)));
        let mut a = activity(ON);
        run(&mut a, cx, "on").unwrap();
        at(T + 5);
        a.on_event(Event::WindowChanged { pid: BRAVE }, cx);
        // The first ask was overtaken: its answer is dropped.
        answer_nth(&mut a, cx, T + 6, 1, TabUrl::Url("https://old.dev/".into()));
        assert_eq!(urled(cx), [brave(T, T + 6, None)]);
        // A private window (or no window) gives no URL.
        at(T + 10);
        a.on_event(Event::WindowChanged { pid: BRAVE }, cx);
        answer_nth(&mut a, cx, T + 11, 0, TabUrl::None);
        assert_eq!(urled(cx), [brave(T, T + 11, None)]);
        // An answer that comes after the browser left the front changes no span...
        at(T + 20);
        a.on_event(Event::WindowChanged { pid: BRAVE }, cx);
        at(T + 30);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        answer(&mut a, cx, T + 31, "https://late.dev/");
        assert_eq!(urled(cx).last().unwrap().2, "Mail");
        assert!(urled(cx).iter().all(|s| s.3.is_none()));
        // ...and an answer while the screen is locked waits for nothing.
        at(T + 40);
        a.on_event(Event::AppActivated { pid: BRAVE }, cx);
        a.on_event(Event::Locked, cx);
        answer(&mut a, cx, T + 41, "https://locked.dev/");
        assert!(urled(cx).iter().all(|s| s.3.as_deref() != Some("https://locked.dev/")));
    });
}

#[test]
fn denied_automation_shows_in_status_until_a_read_works() {
    with_cx(|cx| {
        FRONT.with(|f| f.set(Some(BRAVE)));
        let mut a = activity(ON);
        run(&mut a, cx, "on").unwrap();
        assert!(run(&mut a, cx, "status").unwrap().contains("\nurls: on\n"));
        answer_nth(&mut a, cx, T + 1, 0, TabUrl::Denied);
        assert!(run(&mut a, cx, "status").unwrap().contains("\nurls: no Automation permission for Brave Browser\n"));
        assert_eq!(urled(cx), [brave(T, T + 1, None)]);
        // The Today view says why browser time has no domain.
        let footer = a.open("today", cx).unwrap().footer;
        assert!(footer.contains("no URLs from Brave Browser: allow Flick in Privacy & Security > Automation"), "{footer}");
        at(T + 5);
        a.on_event(Event::WindowChanged { pid: BRAVE }, cx);
        answer_nth(&mut a, cx, T + 6, 0, TabUrl::Failed);
        assert!(run(&mut a, cx, "status").unwrap().contains("\nurls: on\n"));
    });
}

#[test]
fn remote_replies_drop_urls_unless_allowed() {
    for (table, shown) in [(ON, false), ("[activity]\nurls = true\nremote_urls = true", true)] {
        with_cx(|cx| {
            FRONT.with(|f| f.set(Some(BRAVE)));
            let mut a = activity(table);
            run(&mut a, cx, "on").unwrap();
            answer(&mut a, cx, T + 1, "https://github.com/a");
            at(T + 30);
            run(&mut a, cx, "remote allow").unwrap();
            cx.remote = true;
            let spans: serde_json::Value = serde_json::from_str(&json(&mut a, cx, "spans")).unwrap();
            let today: serde_json::Value = serde_json::from_str(&json(&mut a, cx, "today")).unwrap();
            cx.remote = false;
            assert_eq!(spans[0]["url"].is_string(), shown, "{table}");
            assert_eq!(today["top_domains"].as_array().unwrap().is_empty(), !shown, "{table}");
            let local: serde_json::Value = serde_json::from_str(&json(&mut a, cx, "spans")).unwrap();
            assert_eq!(local[0]["url"], "https://github.com/a");
        });
    }
}
