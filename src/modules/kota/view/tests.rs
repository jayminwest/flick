use super::*;
use crate::modules::kota::presence::{Pane, Seen};

fn presence(state: State) -> Presence {
    let pane = Pane { id: "wD:p1".into(), status: "working".into(), title: "Fix the cards".into(), ..Pane::default() };
    let seen = Seen { state, pane: Some(pane), failing: vec!["queue".into(), "memory".into()], ..Seen::default() };
    let mut p = Presence::default();
    p.apply(seen, 1_000, false);
    p
}

#[test]
fn glyphs_and_titles() {
    let all = [
        (State::Thinking, "K…"),
        (State::Blocked, "K!"),
        (State::Idle, "K"),
        (State::Degraded, "K~"),
        (State::Down, "K×"),
        (State::Offline, "K-"),
        (State::Unknown, "K?"),
    ];
    for (state, glyph_want) in all {
        let mut p = Presence::default();
        p.state = state;
        assert_eq!(glyph(&p), glyph_want);
    }
    let mut p = presence(State::Thinking);
    assert_eq!(title(&p, 0), "K…");
    assert_eq!(title(&p, 2), "K… 2");
    p.mark_stale();
    assert_eq!(title(&p, 1), "K? 1");
}

#[test]
fn ages_and_clock_times() {
    assert_eq!(age(100, 100), "<1m");
    assert_eq!(age(100, 50), "<1m");
    assert_eq!(age(0, 59), "<1m");
    assert_eq!(age(0, 240), "4m");
    assert_eq!(age(0, 3 * 3_600 + 5), "3h");
    assert_eq!(age(0, 2 * 86_400), "2d");
    assert_eq!(clock(12 * 3_600 + 3 * 60, 0), "12:03");
    assert_eq!(clock(3_600, -7 * 3_600), "18:00");
    assert_eq!(clock(u64::MAX, 0).len(), 5);
}

#[test]
fn headline_tooltip_and_info_rows() {
    let mut p = presence(State::Degraded);
    assert_eq!(headline(&p, 1_240), "KOTA: degraded · 4m");
    assert_eq!(tooltip(&p, 1_240), "KOTA: degraded · 4m\nFix the cards");
    let rows = ["KOTA: degraded · 4m", "Fix the cards", "Checks: queue, memory failing", "Checked 00:16"];
    assert_eq!(info(&p, 1_240, 0), rows);
    p.mark_stale();
    assert_eq!(info(&p, 1_240, 0)[3], "stale");
    assert_eq!(headline(&p, 1_240), "KOTA: degraded · 4m (stale)");
    let fresh = Presence::default();
    assert_eq!(headline(&fresh, 5), "KOTA: unknown");
    assert_eq!(tooltip(&fresh, 5), "KOTA: unknown");
    assert_eq!(info(&fresh, 5, 0), ["KOTA: unknown", "Not checked yet"]);
}

#[test]
fn the_menu_lists_state_then_actions() {
    let p = presence(State::Idle);
    let menu_rows = menu(&p, 3, 1_000, 0);
    assert_eq!(menu_rows[0], Entry::Info("KOTA: idle · <1m".into()));
    assert_eq!(
        menu_rows[4..],
        [
            Entry::Separator,
            Entry::Open { title: "Ask KOTA…".into(), key: "ask" },
            Entry::Open { title: "Inbox (3 waiting)".into(), key: "inbox" },
            Entry::Pick { title: "Open Dashboard".into(), key: "dash" },
            Entry::Pick { title: "Refresh Now".into(), key: "refresh" },
        ]
    );
    let none = menu(&Presence::default(), 0, 0, 0);
    assert_eq!(none[4], Entry::Open { title: "Inbox".into(), key: "inbox" });
}

#[test]
fn status_as_json_and_text() {
    let mut p = presence(State::Thinking);
    p.errors = vec!["dash: refused".into()];
    let extra = Extra { pending: 2, polling: Polling::Every(60) };
    assert_eq!(
        status_json(&p, extra),
        json!({
            "state": "thinking", "stale": false, "since": 1_000,
            "pane": { "id": "wD:p1", "name": "", "status": "working", "title": "Fix the cards" },
            "failing": ["queue", "memory"], "errors": ["dash: refused"],
            "checked_at": 1_000, "pending": 2, "polling": "every 60 s",
        })
    );
    let text = status_text(&p, extra, 1_000, 0);
    assert!(text.starts_with("KOTA: thinking · <1m\nFix the cards\nChecks: queue, memory failing\nChecked "), "{text}");
    assert!(text.ends_with("\ndash: refused\n2 waiting\npolling: every 60 s"), "{text}");
    let none = Extra { pending: 0, polling: Polling::Off };
    assert_eq!(
        status_json(&Presence::default(), none),
        json!({
            "state": "unknown", "stale": false, "since": null, "pane": null, "failing": [],
            "errors": [], "checked_at": null, "pending": 0, "polling": "off (set a key in [kota] to poll)",
        })
    );
    let on_demand = Extra { pending: 0, polling: Polling::OnDemand };
    assert_eq!(status_text(&Presence::default(), on_demand, 0, 0), "KOTA: unknown\nNot checked yet\npolling: on demand");
}

#[test]
fn asks_show_newest_first_with_their_status() {
    use crate::modules::kota::ask::{Ask, Status};
    let ask = |at: u64, text: &str, status: Status| Ask { id: "k1".into(), text: text.into(), at, status };
    let asks = [
        ask(3_600 * 12 + 180, "what's\non   today?", Status::Sending),
        ask(60, &"x".repeat(81), Status::Sent("queued".into())),
        ask(0, "hi", Status::Failed("ssh: refused".into())),
    ];
    assert_eq!(
        asks_text(&asks, 0),
        format!("12:03  sending…  what's on today?\n00:01  sent  {}…\n00:00  failed: ssh: refused  hi", "x".repeat(80))
    );
    assert_eq!(asks_text(&[], 0), "");
}
