use std::cell::RefCell;
use std::time::Instant;

use super::*;
use crate::config::parse;
use crate::core::test_cx;

mod ink;

thread_local! {
    /// What the fakes saw on this test's thread: `png <len>`, `text <s>`, `open <path>`...
    static CALLS: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}

fn calls() -> Vec<String> {
    CALLS.with(|c| c.borrow_mut().drain(..).collect())
}

fn log(call: String) {
    CALLS.with(|c| c.borrow_mut().push(call));
}

/// A 4x3 PNG header, enough for `png_size`.
fn png() -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend(4u32.to_be_bytes());
    b.extend(3u32.to_be_bytes());
    b.extend([8, 6, 0, 0, 0]);
    b
}

/// A shutter that writes the fixture where screencapture would.
#[expect(clippy::unnecessary_wraps, reason = "it stands in for capture::run")]
fn fake_shoot(req: &Request) -> Result<Shot, Error> {
    std::fs::create_dir_all(req.path.parent().unwrap()).unwrap();
    std::fs::write(&req.path, png()).unwrap();
    Ok(Shot { path: req.path.clone(), width: 4, height: 3 })
}

/// A fresh temp dir for one test.
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("flk-{}-capture-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A module saving into `dir`, whose fakes log to `CALLS`.
fn module(dir: &Path, extra: &str) -> Capture {
    let mut m = Capture::default();
    configure(&mut m, &format!("[capture]\ndir = {:?}\n{extra}", dir.display().to_string())).unwrap();
    m.env.shoot = fake_shoot;
    m.env.set_png = |b| log(format!("png {}", b.len()));
    m.env.set_text = |t| log(format!("text {t}"));
    m.env.open = |p| log(format!("open {}", p.display()));
    m.env.reveal = |p| log(format!("reveal {}", p.display()));
    m
}

fn configure(m: &mut Capture, text: &str) -> Result<(), String> {
    m.configure(&parse(text)?.section("capture")?.unwrap())
}

fn words(w: &[&str]) -> Vec<String> {
    w.iter().map(|s| (*s).to_string()).collect()
}

/// `test_cx` with the capture table in its store.
fn with_cx<R>(f: impl FnOnce(&mut Cx) -> R) -> R {
    test_cx("", |cx| {
        cx.store.migrate("capture", MIGRATIONS).unwrap();
        f(cx)
    })
}

fn settle(m: &Capture) {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while m.worker.busy() && Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn settings_default_and_errors() {
    let m = Capture::default();
    assert!(m.settings.copy && m.settings.save && m.settings.sound && m.settings.shadow);
    assert!(!m.settings.cursor);
    assert_eq!((m.settings.history, m.settings.name.as_str()), (200, name::DEFAULT));
    assert!(m.dir.ends_with("Pictures/Flick"));
    assert!(m.hotkeys().is_empty());
    let mut m = Capture::default();
    for (text, err) in [
        ("[capture]\nnope = 1", "[capture]: unknown field `nope`"),
        ("[capture]\nname = \"a/b\"", "[capture]: name \"a/b\" must not contain '/'"),
        ("[capture]\nhistory = 0", "[capture]: history must be at least 1"),
        ("[capture]\nsave = false\ncopy = false", "[capture]: save and copy are both off"),
        ("[capture]\ncopy = 1", "[capture]: invalid type"),
    ] {
        let e = configure(&mut m, text).unwrap_err();
        assert!(e.starts_with(err), "{e}");
    }
}

#[test]
fn hotkeys_bind_area_window_and_screen() {
    let mut m = Capture::default();
    let text = "[capture]\narea_hotkey = \"cmd+shift+2\"\nwindow_hotkey = \"\"\nscreen_hotkey = \"cmd+shift+3\"";
    configure(&mut m, text).unwrap();
    let keys: Vec<_> = m.hotkeys().into_iter().map(|b| (b.spec, b.key.unwrap())).collect();
    assert_eq!(keys, [("cmd+shift+2".into(), "area".into()), ("cmd+shift+3".into(), "screen".into())]);
    with_cx(|cx| assert!(m.hotkey("other", cx).is_none()));
}

#[test]
fn root_items_have_fixed_ids_and_recent_opens_the_view() {
    let dir = temp("items");
    let mut m = module(&dir, "");
    with_cx(|cx| {
        let ids: Vec<_> = m.items(cx).into_iter().map(|i| i.id.to_string()).collect();
        let want = [
            "capture:area",
            "capture:window",
            "capture:screen",
            "capture:area-annotate",
            "capture:draw",
            "capture:cursor",
            "capture:recent",
        ];
        assert_eq!(ids, want);
        let pushed = m.activate(&ItemId::new("capture", "recent"), cx);
        assert!(matches!(pushed, Outcome::Push(v) if v.is("capture", "recent")));
        assert!(m.open("recent", cx).is_some() && m.open("other", cx).is_none());
        assert!(matches!(m.activate(&ItemId::new("capture", "shot/9"), cx), Outcome::Stay(None)));
        assert!(m.actions(&ItemId::new("capture", "area"), cx).is_empty());
    });
}

#[test]
fn screen_verb_saves_copies_records_and_answers_json() {
    let dir = temp("screen");
    let mut m = module(&dir, "name = \"shot {kind}\"");
    with_cx(|cx| {
        cx.json = true;
        let reply = m.command(&words(&["screen"]), cx).unwrap();
        let path = dir.join("shot screen.png");
        let want = format!(r#"{{"path":{:?},"width":4,"height":3,"copied":true}}"#, path.display().to_string());
        assert_eq!(reply, want);
        assert_eq!(calls(), ["png 29"]);
        // The name is taken: the next one gets a suffix, and --no-copy leaves the clipboard.
        cx.json = false;
        let reply = m.command(&words(&["display", "2", "--no-copy"]), cx).unwrap();
        assert_eq!(reply, dir.join("shot display.png").display().to_string());
        let reply = m.command(&words(&["display", "1", "--no-copy"]), cx).unwrap();
        assert_eq!(reply, dir.join("shot display (2).png").display().to_string());
        assert!(calls().is_empty());
        let out = dir.join("out/rect.png");
        let rect = words(&["rect", "0,0,400,300", "--out", &out.display().to_string()]);
        assert_eq!(m.command(&rect, cx).unwrap(), out.display().to_string());
        let kinds: Vec<_> = cx.store.shots(10).into_iter().map(|r| r.kind).collect();
        assert_eq!(kinds, ["rect", "display", "display", "screen"]);
        assert_eq!(m.command(&words(&["last"]), cx).unwrap(), out.display().to_string());
    });
}

#[test]
fn save_off_copies_a_temp_file_without_a_row() {
    let dir = temp("nosave");
    let mut m = module(&dir, "save = false\nname = \"flk-capture-nosave-{kind}\"");
    with_cx(|cx| {
        let path = PathBuf::from(m.command(&words(&["screen"]), cx).unwrap());
        assert!(path.starts_with(std::env::temp_dir().join("flick-capture")), "{}", path.display());
        assert_eq!(calls(), ["png 29"]);
        assert!(cx.store.shots(10).is_empty());
        std::fs::remove_file(path).unwrap();
    });
}

#[test]
fn command_parsing_errors() {
    let dir = temp("parse");
    let mut m = module(&dir, "");
    with_cx(|cx| {
        let mut err = |w: &[&str]| m.command(&words(w), cx).unwrap_err();
        assert_eq!(err(&["rect", "1,2,3"]), "capture: bad rect \"1,2,3\" (want x,y,w,h)");
        assert_eq!(err(&["rect", "1,2,0,4"]), "capture: bad rect \"1,2,0,4\" (want x,y,w,h)");
        assert_eq!(err(&["rect", "a,b,c,d"]), "capture: bad rect \"a,b,c,d\" (want x,y,w,h)");
        assert_eq!(err(&["display", "0"]), "capture: bad display \"0\"");
        assert_eq!(err(&["screen", "--loud"]), "capture: unknown option \"--loud\"");
        assert_eq!(err(&["screen", "--out"]), "capture: --out needs a path");
        assert_eq!(err(&["screen", "--out", "rel.png"]), "capture: --out needs an absolute path");
        assert_eq!(err(&["ls", "--limit", "x"]), "capture: bad limit \"x\"");
        assert_eq!(err(&["ls", "x"]), "capture: usage: capture ls [--limit n]");
        assert_eq!(err(&["last"]), "capture: no captures yet");
        assert_eq!(err(&["snap"]), "capture: unknown command \"snap\"");
        assert_eq!(err(&[]), "capture: missing command");
    });
}

#[test]
fn without_permission_nothing_is_captured() {
    let dir = temp("denied");
    let mut m = module(&dir, "");
    m.env.permitted = || false;
    with_cx(|cx| {
        let stay = m.activate(&ItemId::new("capture", "area"), cx);
        assert!(matches!(stay, Outcome::Stay(Some(s)) if s == PERMISSION));
        let err = m.command(&words(&["screen"]), cx).unwrap_err();
        assert_eq!(err, format!("capture: {PERMISSION}"));
        assert!(m.command(&words(&["window"]), cx).is_err());
        assert!(m.hotkey("area", cx).is_none());
    });
    assert!(!m.worker.busy() && std::fs::read_dir(&dir).unwrap().next().is_none());
}

#[test]
fn launcher_captures_land_on_module_changed() {
    let dir = temp("async");
    let mut m = module(&dir, "name = \"{kind}\"");
    with_cx(|cx| {
        assert_eq!(show(&m.activate(&ItemId::new("capture", "area"), cx)), "Hide");
        settle(&m);
        assert!(!m.on_event(Event::Wake, cx));
        assert!(!m.on_event(Event::ModuleChanged { module: "flick" }, cx));
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        assert_eq!(calls(), ["png 29"]);
        // Screen from the launcher resolves the display first, then waits for the panel.
        assert_eq!(show(&m.activate(&ItemId::new("capture", "screen"), cx)), "Hide");
        let busy = m.activate(&ItemId::new("capture", "window"), cx);
        assert_eq!(show(&busy), r#"Stay(Some("A capture is already running"))"#);
        settle(&m);
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        let rows: Vec<_> = cx.store.shots(10).into_iter().map(|r| (r.kind, r.width)).collect();
        assert_eq!(rows, [("screen".into(), 4), ("area".into(), 4)]);
    });
}

#[test]
fn cancelled_and_failed_captures_leave_no_row() {
    let dir = temp("cancel");
    let mut m = module(&dir, "");
    with_cx(|cx| {
        m.env.shoot = |_| Err(Error::Cancelled);
        assert_eq!(m.command(&words(&["window"]), cx).unwrap(), "Select a window");
        settle(&m);
        m.env.shoot = |_| Err(Error::Failed("no".into()));
        assert!(m.hotkey("window", cx).is_none());
        settle(&m);
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        assert!(cx.store.shots(10).is_empty() && calls().is_empty());
        m.env.shoot = fake_shoot;
        assert_eq!(m.command(&words(&["area", "--no-copy"]), cx).unwrap(), "Select an area");
        settle(&m);
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        assert!(calls().is_empty());
        assert_eq!(cx.store.shots(10).len(), 1);
    });
}

/// An outcome as text, for short asserts.
fn show(o: &Outcome) -> String {
    format!("{o:?}")
}

#[test]
fn recent_view_lists_live_shots_and_acts() {
    let dir = temp("recent");
    let mut m = module(&dir, "name = \"{kind}\"");
    with_cx(|cx| {
        m.command(&words(&["screen"]), cx).unwrap();
        m.command(&words(&["display", "1"]), cx).unwrap();
        // A row whose file is gone drops out.
        std::fs::remove_file(dir.join("screen.png")).unwrap();
        calls();
        let mut view = m.open("recent", cx).unwrap();
        m.refresh(&mut view, cx);
        let titles: Vec<_> = view.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["display.png"]);
        assert!(view.items[0].subtitle.starts_with("4×3"));
        assert_eq!(cx.store.shots(10).len(), 1);
        let id = view.items[0].id.clone();
        let keys: Vec<_> = m.actions(&id, cx).into_iter().map(|a| a.key).collect();
        assert_eq!(keys, ["copy-image", "annotate", "reveal", "copy-path", "trash"]);
        let path = dir.join("display.png").display().to_string();
        let shown: Vec<_> = [None, Some("copy-image"), Some("copy-path"), Some("reveal"), Some("nope")]
            .into_iter()
            .map(|key| match key {
                None => show(&m.activate(&id, cx)),
                Some(k) => show(&m.act(&id, k, cx)),
            })
            .collect();
        assert_eq!(
            shown,
            ["Hide", r#"Stay(Some("Copied image"))"#, r#"Stay(Some("Copied path"))"#, "Hide", "Stay(None)"]
        );
        assert_eq!(calls(), [format!("open {path}"), "png 29".into(), format!("text {path}"), format!("reveal {path}")]);
    });
}

#[test]
fn move_to_trash_asks_then_drops_the_row() {
    let dir = temp("trash");
    let mut m = module(&dir, "name = \"{kind}\"");
    with_cx(|cx| {
        m.command(&words(&["screen"]), cx).unwrap();
        let id = ItemId::new("capture", format!("shot/{}", cx.store.shots(1)[0].id));
        let Outcome::Confirm(confirm) = m.act(&id, "trash", cx) else { panic!("trash asks first") };
        assert!(confirm.destructive && confirm.label == "Move to Trash");
        assert_eq!(confirm.title, "Move screen.png to the Trash?");
        // The test Trash refuses: the row stays.
        assert_eq!(show(&m.confirmed(&confirm.token, cx)), r#"Stay(Some("tests never use the Trash"))"#);
        m.env.trash = |p| Ok(p.to_path_buf());
        assert_eq!(show(&m.confirmed(&confirm.token, cx)), r#"Stay(Some("Moved to Trash"))"#);
        assert!(cx.store.shots(10).is_empty());
        assert_eq!(show(&m.confirmed(&confirm.token, cx)), "Stay(None)");
        assert_eq!(show(&m.act(&id, "copy-path", cx)), r#"Stay(Some("That capture is gone"))"#);
        let mut view = m.open("recent", cx).unwrap();
        m.refresh(&mut view, cx);
        assert_eq!(view.empty, "No captures yet");
    });
}

#[test]
fn copy_image_reports_a_missing_file() {
    let dir = temp("missing");
    let mut m = module(&dir, "name = \"{kind}\"");
    with_cx(|cx| {
        m.command(&words(&["screen", "--no-copy"]), cx).unwrap();
        let id = ItemId::new("capture", format!("shot/{}", cx.store.shots(1)[0].id));
        std::fs::remove_file(dir.join("screen.png")).unwrap();
        assert!(matches!(m.act(&id, "copy-image", cx), Outcome::Stay(Some(s)) if s.contains("screen.png")));
    });
}

#[test]
fn ls_lists_rows_as_text_or_json() {
    let dir = temp("ls");
    let mut m = module(&dir, "name = \"{kind}\"");
    with_cx(|cx| {
        assert_eq!(m.command(&words(&["ls"]), cx).unwrap(), "");
        m.command(&words(&["screen", "--no-copy"]), cx).unwrap();
        let id = cx.store.shots(1)[0].id;
        let path = dir.join("screen.png").display().to_string();
        assert_eq!(m.command(&words(&["ls", "--limit", "5"]), cx).unwrap(), format!("{id}\t{path}\t4x3"));
        cx.json = true;
        let rows: serde_json::Value = serde_json::from_str(&m.command(&words(&["ls"]), cx).unwrap()).unwrap();
        assert_eq!(rows[0]["path"], path.as_str());
        assert_eq!(rows[0]["kind"], "screen");
        let last: serde_json::Value = serde_json::from_str(&m.command(&words(&["last"]), cx).unwrap()).unwrap();
        assert_eq!(last, rows[0]);
    });
}
