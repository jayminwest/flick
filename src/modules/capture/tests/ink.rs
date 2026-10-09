//! Annotation, drawing on the screen and the cursor halo, against fakes of `platform::ink`.

use std::cell::Cell;

use super::*;
use crate::platform::ink::editor::Done;

thread_local! {
    static DRAWING: Cell<bool> = const { Cell::new(false) };
    static CURSOR: Cell<bool> = const { Cell::new(false) };
    static SHAPES: Cell<bool> = const { Cell::new(false) };
    /// How the fake editor closes; `None` refuses to open.
    static CLOSE: Cell<Option<Done>> = const { Cell::new(Some(Done::Copied)) };
}

/// A module whose ink fakes keep their state in this thread's cells and log to `CALLS`.
fn inky(dir: &Path, extra: &str) -> Capture {
    let mut m = module(dir, extra);
    m.env.ink.set_drawing = |on, style| {
        DRAWING.set(on);
        log(format!("drawing {on} {}", style.palette.len()));
    };
    m.env.ink.drawing = || DRAWING.get();
    m.env.ink.has_shapes = || SHAPES.get();
    m.env.ink.clear = || log("clear".into());
    m.env.ink.set_cursor = |halo| {
        CURSOR.set(halo.is_some());
        log(format!("cursor {:?}", halo.map(|h| h.radius)));
    };
    m.env.ink.cursor = || CURSOR.get();
    m.env.ink.relayout = || log("relayout".into());
    m.env.ink.edit = |src, dest, opts, on_done| {
        let Some(done) = CLOSE.get() else { return false };
        log(format!("edit {} -> {} copy {}", file(src), file(dest), opts.copy));
        if done != Done::Cancelled {
            std::fs::copy(src, dest).unwrap();
        }
        on_done(done);
        true
    };
    m
}

fn file(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into_owned()
}

fn titles(m: &mut Capture, cx: &mut Cx) -> Vec<String> {
    m.items(cx).into_iter().filter(|i| i.id.key() != "area").map(|i| i.title).collect()
}

#[test]
fn ink_settings_and_errors() {
    let mut m = Capture::default();
    assert_eq!(m.ink.style.palette.len(), 5);
    assert!((m.ink.halo.radius - 28.0).abs() < f64::EPSILON);
    let text = "[capture]\ncolors = [\"#000000\", \"#ffffff80\"]\nwidth = 2.5\nfade_secs = 3.0\nhalo_radius = 10.0";
    configure(&mut m, text).unwrap();
    assert_eq!(m.ink.style.palette.len(), 2);
    assert!((m.ink.style.palette[1][3] - 128.0 / 255.0).abs() < 1e-6);
    assert!((m.ink.style.fade_secs - 3.0).abs() < f32::EPSILON);
    for (text, err) in [
        ("colors = []", "[capture]: colors needs 1 to 5 entries"),
        ("colors = [\"red\"]", "[capture]: bad color \"red\""),
        ("halo_color = \"#12\"", "[capture]: bad color \"#12\""),
        ("width = 0.0", "[capture]: width must be above 0"),
        ("fade_secs = -1.0", "[capture]: fade_secs must be 0 or more"),
        ("halo_radius = 0.0", "[capture]: halo_radius must be above 0"),
    ] {
        let e = configure(&mut m, &format!("[capture]\n{text}")).unwrap_err();
        assert!(e.starts_with(err), "{e}");
    }
}

#[test]
fn hotkeys_bind_annotate_draw_and_cursor() {
    let dir = temp("ink-hotkeys");
    let text = "annotate_hotkey = \"cmd+shift+4\"\ndraw_hotkey = \"cmd+shift+d\"\ncursor_hotkey = \"cmd+shift+c\"";
    let mut m = inky(&dir, text);
    let keys: Vec<_> = m.hotkeys().into_iter().map(|b| b.key.unwrap()).collect();
    assert_eq!(keys, ["annotate", "draw", "cursor"]);
    with_cx(|cx| {
        assert!(m.hotkey("draw", cx).is_none() && DRAWING.get());
        assert!(m.hotkey("draw", cx).is_none() && !DRAWING.get());
        assert!(m.hotkey("cursor", cx).is_none() && CURSOR.get());
        assert_eq!(calls(), ["drawing true 5", "drawing false 5", "cursor Some(28.0)"]);
        assert!(m.hotkey("annotate", cx).is_none());
        settle(&m);
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        // The editor copies, so the shutter does not.
        assert_eq!(calls().len(), 1);
    });
}

#[test]
fn items_follow_the_overlay_and_toggle_it() {
    let dir = temp("ink-items");
    let mut m = inky(&dir, "");
    with_cx(|cx| {
        let off = titles(&mut m, cx);
        assert_eq!(off[2..5], ["Capture Area and Annotate", "Draw on Screen", "Highlight Cursor"]);
        for key in ["draw", "cursor"] {
            assert_eq!(show(&m.activate(&ItemId::new("capture", key), cx)), "Hide");
        }
        SHAPES.set(true);
        let on = titles(&mut m, cx);
        assert_eq!(on[3..6], ["Stop Drawing on Screen", "Stop Highlighting Cursor", "Clear Drawing"]);
        assert_eq!(show(&m.activate(&ItemId::new("capture", "clear"), cx)), "Hide");
        assert_eq!(show(&m.activate(&ItemId::new("capture", "draw"), cx)), "Hide");
        assert_eq!(calls(), ["drawing true 5", "cursor Some(28.0)", "clear", "drawing false 5"]);
        assert!(!m.on_event(Event::DisplaysChanged, cx));
        assert_eq!(calls(), ["relayout"]);
    });
    SHAPES.set(false);
}

#[test]
fn draw_and_cursor_verbs() {
    let dir = temp("ink-verbs");
    let mut m = inky(&dir, "");
    with_cx(|cx| {
        let mut run = |w: &[&str]| m.command(&words(w), cx);
        assert_eq!(run(&["draw", "toggle"]).unwrap(), "Drawing on the screen");
        assert_eq!(run(&["draw", "off"]).unwrap(), "Stopped drawing");
        assert_eq!(run(&["draw", "on"]).unwrap(), "Drawing on the screen");
        assert_eq!(run(&["draw", "clear"]).unwrap(), "Cleared the drawing");
        assert_eq!(run(&["cursor", "on"]).unwrap(), "Highlighting the cursor");
        assert_eq!(run(&["cursor", "toggle"]).unwrap(), "Stopped highlighting the cursor");
        assert_eq!(run(&["cursor", "off"]).unwrap(), "Stopped highlighting the cursor");
        assert_eq!(run(&["draw", "x"]).unwrap_err(), "capture: usage: capture draw on|off|toggle|clear");
        assert_eq!(run(&["cursor", "x"]).unwrap_err(), "capture: usage: capture cursor on|off|toggle");
        assert_eq!(run(&["screen", "--annotate"]).unwrap_err(), "capture: --annotate works with area and window");
        let want = ["drawing true 5", "drawing false 5", "drawing true 5", "clear", "cursor Some(28.0)"];
        assert_eq!(calls()[..5], want);
    });
}

#[test]
fn capture_area_and_annotate_edits_the_shot_in_place() {
    let dir = temp("ink-area");
    let mut m = inky(&dir, "name = \"{kind}\"");
    with_cx(|cx| {
        assert_eq!(show(&m.activate(&ItemId::new("capture", "area-annotate"), cx)), "Hide");
        settle(&m);
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        assert_eq!(calls(), ["edit area.png -> area.png copy true"]);
        // One row: the edit saved over the shot.
        assert_eq!(cx.store.shots(10).len(), 1);
        assert_eq!(m.command(&words(&["area", "--annotate", "--no-copy"]), cx).unwrap(), "Select an area");
        settle(&m);
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        assert_eq!(calls(), ["edit area (2).png -> area (2).png copy false"]);
        // An editor that will not open is logged; the shot stays recorded.
        CLOSE.set(None);
        assert!(m.hotkey("annotate", cx).is_none());
        settle(&m);
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        assert!(calls().is_empty());
        assert_eq!(cx.store.shots(10).len(), 3);
    });
    CLOSE.set(Some(Done::Copied));
}

#[test]
fn annotate_records_a_copy_beside_the_source() {
    let dir = temp("ink-copy");
    let mut m = inky(&dir, "name = \"{kind}\"");
    with_cx(|cx| {
        m.command(&words(&["screen", "--no-copy"]), cx).unwrap();
        let id = ItemId::new("capture", format!("shot/{}", cx.store.shots(1)[0].id));
        assert_eq!(show(&m.act(&id, "annotate", cx)), "Hide");
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        assert_eq!(calls(), ["edit screen.png -> screen annotated.png copy true"]);
        let rows: Vec<_> = cx.store.shots(10).into_iter().map(|r| (file(Path::new(&r.path)), r.kind)).collect();
        assert_eq!(rows[0], ("screen annotated.png".into(), "annotated".into()));
        // The verb answers with the copy's path; a cancelled edit records nothing.
        CLOSE.set(Some(Done::Cancelled));
        let src = dir.join("screen.png").display().to_string();
        let dest = m.command(&words(&["annotate", &src]), cx).unwrap();
        assert_eq!(dest, dir.join("screen annotated (2).png").display().to_string());
        assert!(m.on_event(Event::ModuleChanged { module: "capture" }, cx));
        assert_eq!(cx.store.shots(10).len(), 2);
        calls();
        CLOSE.set(None);
        let refused = m.act(&id, "annotate", cx);
        assert!(matches!(refused, Outcome::Stay(Some(e)) if e.starts_with("capture: cannot annotate")));
        let missing = dir.join("nope.png").display().to_string();
        assert_eq!(m.command(&words(&["annotate", &missing]), cx).unwrap_err(), format!("capture: no file at {missing}"));
    });
    CLOSE.set(Some(Done::Copied));
}
