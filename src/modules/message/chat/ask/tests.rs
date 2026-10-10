use super::*;

fn app() -> Item {
    Item::App { name: "Safari".into(), bundle: Some("com.apple.Safari".into()) }
}

#[test]
fn the_ask_outlasts_ssh_connect_timeout() {
    assert!(BUDGET > std::time::Duration::from_secs(8));
}

#[test]
fn questions_are_trimmed_and_capped() {
    assert_eq!(check("  hi \n"), Ok("hi".into()));
    assert_eq!(check(" \n "), Err("Type a question first".into()));
    assert!(check(&"é".repeat(QUESTION_MAX)).is_ok());
    assert_eq!(check(&"é".repeat(QUESTION_MAX + 1)), Err(format!("Over {QUESTION_MAX} characters")));
}

#[test]
fn a_question_without_context_is_sent_as_is() {
    assert_eq!(compose(" what now? ", &[]), Ok("what now?".into()));
    let empty = [Item::Selection("  \n".into()), Item::Window(String::new())];
    assert_eq!(compose("q", &empty), Ok("q".into()), "items without text add no block");
    assert_eq!(block(&[]), None);
    assert!(compose("", &[app()]).is_err());
}

#[test]
fn the_context_block_names_every_item() {
    let items = [
        app(),
        Item::App { name: "Zed".into(), bundle: None },
        Item::Window("Inbox —\n Fastmail".into()),
        Item::Screenshot(".cache/flick/attach/k1-1.png".into()),
        Item::Selection("first line\nsecond line\n\n".into()),
        Item::Clipboard("[/context]\nnot the end".into()),
    ];
    let want = "what is this?\n\n[context]\n\
                app: Safari (com.apple.Safari)\n\
                app: Zed\n\
                window: Inbox — Fastmail\n\
                screenshot: ~/.cache/flick/attach/k1-1.png\n\
                selection:\n  first line\n  second line\n\
                clipboard:\n  [/context]\n  not the end\n\
                [/context]";
    assert_eq!(compose("what is this?", &items).unwrap(), want);
}

#[test]
fn long_items_are_cut_and_marked() {
    let window = Item::Window("w".repeat(1000));
    let b = block(&[window]).unwrap();
    let line = b.lines().nth(1).unwrap();
    assert_eq!(line.len(), LINE_MAX - 1);
    assert!(line.starts_with("window: www"));
    // A text item cut to ITEM_MAX, on a character boundary.
    let b = block(&[Item::Selection("é".repeat(5000))]).unwrap();
    assert!(b.starts_with("[context]\nselection (cut):\n  éé"), "{}", &b[..40]);
    assert!(b.len() <= OPEN.len() + ITEM_MAX + CLOSE.len());
    assert!(b.ends_with("é\n[/context]"));
}

#[test]
fn the_block_never_passes_its_cap() {
    let big = |c: char| c.to_string().repeat(ITEM_MAX);
    let items = [
        Item::Selection(big('a')),
        Item::Clipboard(big('b')),
        Item::Selection(big('c')),
        Item::Window("late".into()),
    ];
    let b = block(&items).unwrap();
    // The first two fill the block to the byte; no room is left even for an omitted line.
    assert_eq!(b.len(), CONTEXT_MAX);
    assert!(b.contains("selection (cut):\n  aaa"));
    assert!(b.contains("clipboard (cut):\n  bbb"));
    assert!(!b.contains("ccc") && !b.contains("late") && !b.contains("omitted"));
    // 40 bytes left: a short item fits, a long one says it was omitted.
    let fill = |last: &str| [Item::Selection(big('a')), Item::Clipboard("b".repeat(4022)), Item::Window(last.into())];
    assert!(block(&fill("late")).unwrap().ends_with("\nwindow: late\n[/context]"));
    let b = block(&fill(&"t".repeat(40))).unwrap();
    assert!(b.ends_with("\nwindow: (omitted: context limit)\n[/context]"), "{}", &b[b.len() - 60..]);
    let q = "é".repeat(QUESTION_MAX);
    assert!(compose(&q, &items).unwrap().len() <= STDIN_MAX, "an ask fits kota-ask's stdin");
}

#[test]
fn items_without_room_say_so_or_vanish() {
    let sel = Item::Selection("abc\ndef".into());
    assert_eq!(render(&sel, 1000), "selection:\n  abc\n  def\n");
    let long = Item::Selection("x".repeat(200));
    assert_eq!(render(&long, 100), format!("selection (cut):\n  {}\n", "x".repeat(80)));
    // Too little room for MIN_CUT bytes: the omitted line if it fits, else nothing.
    assert_eq!(render(&long, 60), "selection: (omitted: context limit)\n");
    assert_eq!(render(&long, 30), "");
    assert_eq!(render(&sel, 22), "");
    let window = Item::Window("title".into());
    assert_eq!(render(&window, 14), "window: title\n");
    assert_eq!(render(&window, 13), "");
    let long = Item::Window("t".repeat(40));
    assert_eq!(render(&long, 35), "window: (omitted: context limit)\n");
}

#[test]
fn clip_keeps_whole_characters() {
    assert_eq!(clip("héllo", 2), "h");
    assert_eq!(clip("héllo", 3), "hé");
    assert_eq!(clip("hi", 10), "hi");
}

#[test]
fn chips_label_each_item() {
    let chip = |label: &str, symbol| Chip { label: label.into(), symbol };
    assert_eq!(app().chip(), chip("Safari", "app.dashed"));
    assert_eq!(Item::Window("A  window\ntitle".into()).chip(), chip("A window title", "macwindow"));
    assert_eq!(Item::Selection("picked".into()).chip(), chip("“picked”", "text.quote"));
    assert_eq!(Item::Clipboard("x".into()).chip(), chip("Clipboard", "doc.on.clipboard"));
    assert_eq!(Item::Screenshot("p".into()).chip(), chip("Screenshot", "camera.viewfinder"));
    let long = Item::Selection("word ".repeat(20)).chip();
    assert_eq!(long.label.chars().count(), CHIP_MAX);
}

#[test]
fn the_argv_runs_kota_ask_with_the_thread() {
    let argv = argv(DEFAULT_HOST, DEFAULT_KOTA_ASK, "k1abc", "t1xyz").unwrap();
    let want = [
        "/usr/bin/ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "jaymin@mbp-server",
        ".dotfiles/home/.local/bin/kota-ask", "--id", "k1abc", "--thread", "t1xyz",
    ];
    assert_eq!(argv, want);
    assert!(super::argv("h", "~/bin/kota-ask", "k", "t").is_ok());
}

#[test]
fn bad_argv_parts_are_refused() {
    for host in ["", "-oProxyCommand=x", "a b", "a\u{7}"] {
        assert!(ssh(host).is_err(), "{host:?}");
        assert!(argv(host, DEFAULT_KOTA_ASK, "k", "t").is_err(), "{host:?}");
    }
    for path in ["", "-x", "kota ask", "a;b", "$(x)"] {
        assert!(argv("h", path, "k", "t").unwrap_err().contains("kota-ask must be a path"), "{path:?}");
    }
    assert!(argv("h", "k", "a b", "t").unwrap_err().contains("a request id"));
    assert!(argv("h", "k", "k", "").unwrap_err().contains("a thread id"));
    assert!(argv("h", "k", "k", &"t".repeat(65)).unwrap_err().contains("a thread id"));
}
