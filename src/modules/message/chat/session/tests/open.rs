//! Opening the window from the threads view and from other modules (`message:chat:open`).

use super::*;

#[test]
fn the_threads_view_lists_and_opens_threads() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "a", "**Plan** for today?"]).unwrap();
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "b", "Two calls."]).unwrap();
    f.run(&mut m, false, &["post", "--thread", "t2", "--id", "c", "Hm"]).unwrap();
    take_log();
    let list = ItemId::new("message", "list");
    let actions = f.cx("", false, |cx| m.actions(&list, cx));
    assert_eq!(actions.iter().map(|a| a.key).collect::<Vec<_>>(), ["threads"]);
    let Outcome::Push(mut view) = f.cx("", false, |cx| m.act(&list, "threads", cx)) else { panic!("no view") };
    assert_eq!(view.name, "threads");
    let mut opened = f.cx("", false, |cx| m.open("threads", cx)).unwrap();
    assert_eq!(opened.empty, "No chat threads yet");
    f.cx("", false, |cx| m.refresh(&mut view, cx));
    let rows: Vec<String> = view.items.iter().map(|i| format!("{} {} {}", i.id, i.title, i.subtitle)).collect();
    assert_eq!(rows, ["message:thread:t2 Hm 11:31  ·  1 message", "message:thread:t1 Plan for today? 11:31  ·  2 messages"]);
    f.cx("plan", false, |cx| m.refresh(&mut opened, cx));
    assert_eq!(opened.items.len(), 1);
    let id = ItemId::new("message", "thread:t1");
    assert!(matches!(f.cx("", false, |cx| m.activate(&id, cx)), Outcome::Hide));
    assert_eq!(m.chat_showing(), Some("t1"));
    let log = take_log();
    assert_eq!(log.first().map(String::as_str), Some("launcher hide"));
    assert_eq!(only("chat header", &log), ["Plan for today?|⌘N new thread  ·  ⌘[ ⌘] switch  ·  Esc hides|idle"]);
    assert!(matches!(f.cx("", false, |cx| m.act(&list, "other", cx)), Outcome::Stay(None)));
}

#[test]
fn chat_open_shows_the_window_and_never_hides_it() {
    // `message:chat:open` is how other modules open the window (kota's Open Chat, flick-ed63).
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "a", "Plan?"]).unwrap();
    take_log();
    let id = ItemId::new("message", "chat:open");
    assert!(f.cx("", false, |cx| m.actions(&id, cx)).is_empty());
    assert!(matches!(f.cx("", false, |cx| m.activate(&id, cx)), Outcome::Hide));
    assert_eq!(m.chat_showing(), Some("t1"), "the newest thread");
    assert_eq!(take_log().first().map(String::as_str), Some("launcher hide"));
    // The hotkey key of the same name (the KOTA menu's row) shows it again, keyboard and all.
    for _ in 0..2 {
        assert!(f.cx("", false, |cx| m.hotkey("chat:open", cx)).is_none());
        assert_eq!(m.chat_showing(), Some("t1"));
        assert_eq!(take_log().last().map(String::as_str), Some("chat show"));
    }
}
