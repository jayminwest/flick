//! flick-5dfe: deleting a chat, the context cap, the kept model pick and a stored chat whose
//! server left the config.

use super::*;

/// Keep message `body` as seq 0 of thread `id` on `server`, as an older Flick did.
fn keep(f: &Fx, id: &str, server: &str, body: &str) {
    let msg = Msg { who: Who::User, model: "old-model".into(), body: body.into(), end: End::Done, ts: 5 };
    f.store.llm_save(&Save { id, server, title: body, seq: 0, msg: &msg, keep: 10 }).unwrap();
}

/// The messages of the last request to `url` whose body holds `mark` (tests share `SENT`).
fn sent_messages(url: &str, mark: &str) -> Vec<String> {
    let req = sent(url).into_iter().rev().find(|s| String::from_utf8_lossy(&s.stdin).contains(mark)).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&req.stdin).unwrap();
    body["messages"].as_array().unwrap().iter().map(|t| t["content"].as_str().unwrap().to_string()).collect()
}

fn thread_item(id: &str) -> ItemId {
    ItemId::new(ID, format!("thread:{id}"))
}

#[test]
fn delete_chat_asks_first_then_drops_the_thread() {
    let mut f = Fx::new();
    let mut m = llm(&format!("[llm]\ndefault_model = \"q\"\n{MLX}"));
    hotkey(&mut f, &mut m);
    send(&mut f, &mut m, "shown chat");
    let shown = m.chat.thread.as_ref().unwrap().id.clone();
    keep(&f, "old", "mlx", "an old chat");
    // ⌘K on a kept thread: Delete Chat, which asks first (⌘↵ only).
    let keys: Vec<&str> = f.cx("", |cx| m.actions(&thread_item("old"), cx)).iter().map(|a| a.key).collect();
    assert_eq!(keys, ["delete"]);
    let Outcome::Confirm(c) = f.cx("", |cx| m.act(&thread_item("old"), "delete", cx)) else { panic!("delete asks first") };
    assert_eq!((c.module, c.token.as_str(), c.title.as_str()), (ID, "delete:old", "Delete chat \"an old chat\"?"));
    assert_eq!((c.label.as_str(), c.destructive), ("Delete Chat", true));
    assert_eq!((c.rows[0].title.as_str(), c.rows[0].subtitle.as_str()), ("an old chat", "mlx · old-model  ·  1 message"));
    // Confirmed: gone with its messages; the shown chat stays.
    take_log();
    assert!(matches!(f.cx("", |cx| m.confirmed(&c.token, cx)), Outcome::Stay(Some(s)) if s == "Deleted the chat"));
    assert!(f.store.llm_thread("old").is_none());
    assert_eq!(m.chat.thread.as_ref().unwrap().id, shown);
    assert!(take_log().is_empty());
}

#[test]
fn deleting_the_shown_chat_starts_a_new_one_and_odd_cases_say_why() {
    let mut f = Fx::new();
    let mut m = llm(&format!("[llm]\ndefault_model = \"q\"\n{MLX}"));
    hotkey(&mut f, &mut m);
    send(&mut f, &mut m, "shown chat");
    let shown = m.chat.thread.as_ref().unwrap().id.clone();
    // Deleting the shown chat: the window starts a new one.
    let Outcome::Confirm(c) = f.cx("", |cx| m.act(&thread_item(&shown), "delete", cx)) else { panic!() };
    f.cx("", |cx| m.confirmed(&c.token, cx));
    assert_eq!(f.store.llm_threads(10), []);
    assert_eq!(last("header"), "New chat|mlx · q  ·  ⌘N new chat  ·  Esc hides|Idle");
    // Already gone, or not a delete: nothing happens.
    let stays = [
        f.cx("", |cx| m.act(&thread_item("old"), "delete", cx)),
        f.cx("", |cx| m.confirmed("delete:old", cx)),
        f.cx("", |cx| m.confirmed("nope", cx)),
        f.cx("", |cx| m.act(&ItemId::new(ID, "chat"), "delete", cx)),
    ]
    .map(|o| match o {
        Outcome::Stay(s) => s,
        _ => Some("not a stay".into()),
    });
    let gone = Some("Chat old is no longer kept".to_string());
    assert_eq!(stays, [gone.clone(), gone, None, None]);
    // flick.db refuses: the error says why.
    f.store.conn().execute_batch("DROP TABLE llm_messages").unwrap();
    let err = f.cx("", |cx| m.confirmed("delete:x", cx));
    assert!(matches!(err, Outcome::Stay(Some(s)) if s.starts_with("llm: can't delete the chat: ")));
}

#[test]
fn deleting_the_chat_mid_reply_writes_nothing_after() {
    let mut f = Fx::new();
    let mut m = llm("[llm]\ndefault_model = \"q\"\n[[llm.servers]]\nname = \"half\"\nurl = \"http://half\"\n");
    hotkey(&mut f, &mut m);
    queue(Note::Submit("go".into()));
    event(&mut f, &mut m);
    let id = m.chat.thread.as_ref().unwrap().id.clone();
    assert!(m.chat.reply.is_some());
    (UI.hide)();
    f.cx("", |cx| m.confirmed(&format!("delete:{id}"), cx));
    assert!(m.chat.reply.is_none());
    event(&mut f, &mut m);
    assert_eq!(f.store.llm_threads(10), []);
    assert!(m.chat.thread.as_ref().is_some_and(|t| t.id != id && t.msgs.is_empty()));
}

#[test]
fn only_the_newest_messages_that_fit_context_chars_are_sent() {
    let mut f = Fx::new();
    // "Hello, world." is 13 characters: 40 fits one exchange and the next prompt.
    let mut m = llm(&format!("[llm]\ndefault_model = \"q\"\ncontext_chars = 40\nsystem_prompt = \"sys\"\n{MLX}"));
    hotkey(&mut f, &mut m);
    send(&mut f, &mut m, "first cap-1");
    send(&mut f, &mut m, "cap-2");
    let url = "http://mlx/v1/chat/completions";
    assert_eq!(sent_messages(url, "cap-2"), ["sys", "first cap-1", "Hello, world.", "cap-2"]);
    take_log();
    send(&mut f, &mut m, "cap-3");
    assert_eq!(sent_messages(url, "cap-3"), ["sys", "cap-2", "Hello, world.", "cap-3"]);
    let notice = only("notice", &take_log()).pop();
    assert_eq!(notice.as_deref(), Some("Sent the newest 3 of 5 messages ([llm] context_chars)"));
    // The thread and flick.db keep every message.
    assert_eq!(m.chat.thread.as_ref().unwrap().msgs.len(), 6);
    assert_eq!(f.store.llm_threads(1)[0].messages, 6);
    // 0: the whole chat (the kept one reopens after the restart).
    let mut m = llm(&format!("[llm]\ndefault_model = \"q\"\ncontext_chars = 0\n{MLX}"));
    hotkey(&mut f, &mut m);
    send(&mut f, &mut m, "cap-4");
    assert_eq!(sent_messages(url, "cap-4").len(), 7);
}

#[test]
fn the_model_pick_is_kept_across_a_restart() {
    let mut f = Fx::new();
    let (mut m, view) = picker(&mut f);
    let gemma = view.items[1].id.clone();
    f.cx("", |cx| m.activate(&gemma, cx));
    drop(m);
    assert_eq!(f.store.llm_pick(), Some(("mlx".into(), "gemma-3-12b-it-4bit".into())));
    // Flick restarted: the root item, the picker and a new chat use the kept pick.
    let mut m = llm(&format!("{MLX}{VAULT}"));
    assert_eq!(f.cx("", |cx| m.items(cx))[0].subtitle, "mlx · gemma-3-12b-it-4bit");
    queue(Note::Key(Command::New));
    event(&mut f, &mut m);
    assert_eq!(m.chat.thread.as_ref().unwrap().model, "gemma-3-12b-it-4bit");
    // A pick whose server left the config (or went private) is not used.
    for gone in [String::from("[[llm.servers]]\nname = \"ollama\"\nurl = \"http://mlx\"\n"), format!("{VAULT}{MLX}")] {
        let server = if gone.contains("vault") { "vault" } else { "mlx" };
        f.store.llm_set_pick(server, "gemma").unwrap();
        let mut m = llm(&format!("[llm]\ndefault_model = \"q\"\n{gone}"));
        let item = f.cx("", |cx| m.items(cx)).remove(0);
        assert!(item.subtitle.ends_with(" · q"), "{}", item.subtitle);
    }
    // A pick made before the store was read wins over the kept one.
    f.store.llm_set_pick("mlx", "kept").unwrap();
    let mut m = llm(MLX);
    f.cx("", |cx| m.pick("mlx", "new", cx));
    assert_eq!(f.cx("", |cx| m.items(cx))[0].subtitle, "mlx · new");
    // flick.db refuses the write: the pick holds until quit and the notice says why.
    f.store.conn().execute_batch("DROP TABLE llm_pick").unwrap();
    f.cx("", |cx| m.pick("mlx", "again", cx));
    assert!(m.chat.notice.as_deref().is_some_and(|n| n.starts_with("llm: can't keep the model pick: ")));
}

#[test]
fn the_picker_never_picks_a_private_server() {
    let mut f = Fx::new();
    let (mut m, _) = picker(&mut f);
    let vault = ItemId::new(ID, "model:vault/qwen3-30b-a3b-4bit");
    assert!(matches!(f.cx("", |cx| m.activate(&vault, cx)), Outcome::Stay(None)));
    assert_eq!(m.chat.pick, None);
}

#[test]
fn a_kept_chat_whose_server_left_moves_to_the_default_one() {
    let mut f = Fx::new();
    let mut m = llm(&format!("[llm]\ndefault_model = \"q\"\n{MLX}{VAULT}"));
    for (gone, why) in [("old", "llm: no server \"old\""), ("vault", "llm: vault is private; only the private chat talks to it")] {
        keep(&f, gone, gone, "hi");
        take_log();
        f.cx("", |cx| m.activate(&thread_item(gone), cx));
        let log = take_log();
        assert_eq!(only("notice", &log), [format!("{why}; this chat now uses mlx")]);
        assert_eq!(only("header", &log).last().unwrap(), "hi|mlx · q  ·  ⌘N new chat  ·  Esc hides|Idle");
        send(&mut f, &mut m, "moved-0e41");
        assert_eq!(sent_messages("http://mlx/v1/chat/completions", "moved-0e41").last().unwrap(), "moved-0e41");
        let (t, _) = f.store.llm_thread(gone).unwrap();
        assert_eq!((t.server.as_str(), t.model.as_str()), ("mlx", "q"));
    }
    // Nothing of the chat ever went to the private server.
    let private = sent("http://mlx:11235/v1/chat/completions");
    assert!(!private.iter().any(|s| String::from_utf8_lossy(&s.stdin).contains("moved-0e41")));
}

#[test]
fn a_reload_while_the_chat_shows_moves_it_on_the_next_prompt() {
    let mut f = Fx::new();
    let alt = "[[llm.servers]]\nname = \"alt\"\nurl = \"http://mlx\"\n";
    let mut m = llm(&format!("[llm]\ndefault_model = \"q\"\n{MLX}{alt}"));
    hotkey(&mut f, &mut m);
    f.cx("", |cx| m.pick("alt", "a", cx));
    m.configure(&parse(&format!("[llm]\ndefault_model = \"q\"\n{MLX}")).unwrap().section(ID).unwrap().unwrap()).unwrap();
    take_log();
    send(&mut f, &mut m, "after reload");
    let t = m.chat.thread.as_ref().unwrap();
    assert_eq!((t.server.as_str(), t.model.as_str(), t.msgs.len()), ("mlx", "q", 2));
    assert_eq!(only("notice", &take_log()).last().unwrap(), "llm: no server \"alt\"; this chat now uses mlx");
}

#[test]
fn with_no_normal_server_left_a_prompt_goes_back() {
    let mut f = Fx::new();
    let mut m = llm(VAULT);
    queue(Note::Submit("nowhere".into()));
    event(&mut f, &mut m);
    assert_eq!(only("input", &take_log()), ["nowhere"]);
    assert_eq!(m.chat.notice.as_deref(), Some("llm: every server is private"));
    assert!(m.chat.waiting.is_none());
}

#[test]
fn a_waiting_prompt_with_no_list_yet_asks_for_one() {
    let mut f = Fx::new();
    let mut m = llm(MLX);
    hotkey(&mut f, &mut m);
    assert!(m.shared.lock().models.is_empty());
    m.chat.waiting = Some("list-first".into());
    event(&mut f, &mut m);
    assert!(m.shared.lock().models.contains_key("mlx"));
    settle(&mut f, &mut m);
    assert_eq!(m.chat.thread.as_ref().unwrap().msgs.len(), 2);
}
