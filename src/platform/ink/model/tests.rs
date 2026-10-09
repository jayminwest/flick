use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn near(a: Point, b: Point) -> bool {
    close(a.x, b.x) && close(a.y, b.y)
}

fn same_rgba(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| close(f64::from(*x), f64::from(y)))
}

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// A canvas with one finished shape of `tool` from (0,0) to (10,10), started at `at`.
fn drawn(canvas: &mut Canvas, tool: Tool, at: f64) -> bool {
    canvas.begin(tool, Color(1), pt(0.0, 0.0), at);
    canvas.drag(pt(10.0, 10.0));
    canvas.end()
}

#[test]
fn style_default_and_palette_lookup() {
    let style = Style::default();
    assert_eq!(style.palette.len(), 5);
    assert!(same_rgba(style.rgba(Color(4)), [1.0, 1.0, 1.0, 1.0]));
    assert!(same_rgba(style.rgba(Color(5)), [1.0, 0.0, 0.0, 1.0]));
    let empty = Style { palette: Vec::new(), ..Style::default() };
    assert!(same_rgba(empty.rgba(Color(0)), [1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn begin_starts_a_shape_with_canvas_width() {
    let mut c = Canvas::new(6.0);
    assert!(c.is_empty());
    c.begin(Tool::Rect, Color(2), pt(3.0, 4.0), 7.0);
    let shape = c.active().unwrap();
    assert_eq!((shape.tool, shape.color, shape.text.as_deref()), (Tool::Rect, Color(2), None));
    assert!(close(f64::from(shape.width), 6.0) && close(shape.at, 7.0));
    assert_eq!(shape.points, vec![pt(3.0, 4.0)]);
    assert!(!c.is_empty());
    assert_eq!(c.visible().count(), 1);
}

#[test]
fn text_tool_starts_no_drag() {
    let mut c = Canvas::new(4.0);
    c.begin(Tool::Pen, Color(0), pt(0.0, 0.0), 0.0);
    c.begin(Tool::Text, Color(0), pt(0.0, 0.0), 0.0);
    assert!(c.active().is_none());
    c.drag(pt(5.0, 5.0));
    assert!(!c.end());
    assert!(c.shapes().is_empty());
}

#[test]
fn freehand_drag_skips_points_within_one_point() {
    for tool in [Tool::Pen, Tool::Highlight] {
        let mut c = Canvas::new(4.0);
        c.begin(tool, Color(0), pt(0.0, 0.0), 0.0);
        c.drag(pt(0.5, 0.5));
        c.drag(pt(1.0, 0.0));
        c.drag(pt(2.0, 0.0));
        c.drag(pt(2.0, 3.0));
        assert_eq!(c.active().unwrap().points, vec![pt(0.0, 0.0), pt(2.0, 0.0), pt(2.0, 3.0)]);
        assert!(c.end());
        assert_eq!(c.shapes().len(), 1);
    }
}

#[test]
fn freehand_click_is_kept_as_a_dot() {
    let mut c = Canvas::new(4.0);
    c.begin(Tool::Pen, Color(0), pt(1.0, 1.0), 0.0);
    assert!(c.end());
    assert_eq!(c.shapes()[0].points, vec![pt(1.0, 1.0)]);
}

#[test]
fn two_point_tools_keep_start_and_latest_end() {
    for tool in [Tool::Arrow, Tool::Rect, Tool::Redact] {
        let mut c = Canvas::new(4.0);
        c.begin(tool, Color(0), pt(1.0, 1.0), 0.0);
        c.drag(pt(5.0, 5.0));
        c.drag(pt(9.0, 2.0));
        assert_eq!(c.active().unwrap().points, vec![pt(1.0, 1.0), pt(9.0, 2.0)]);
        assert!(c.end());
        assert!(c.active().is_none());
        assert_eq!(c.shapes()[0].tool, tool);
    }
}

#[test]
fn zero_size_two_point_shapes_are_dropped() {
    for tool in [Tool::Arrow, Tool::Rect, Tool::Redact] {
        let mut c = Canvas::new(4.0);
        c.begin(tool, Color(0), pt(1.0, 1.0), 0.0);
        assert!(!c.end(), "{tool:?} click");
        c.begin(tool, Color(0), pt(1.0, 1.0), 0.0);
        c.drag(pt(1.5, 0.5));
        assert!(!c.end(), "{tool:?} sub-point drag");
        assert!(c.is_empty());
    }
    let mut c = Canvas::new(4.0);
    c.begin(Tool::Arrow, Color(0), pt(0.0, 0.0), 0.0);
    c.drag(pt(0.0, 1.0));
    assert!(c.end(), "a vertical arrow has length");
}

#[test]
fn drag_and_end_without_begin_do_nothing() {
    let mut c = Canvas::new(4.0);
    c.drag(pt(1.0, 1.0));
    assert!(!c.end());
    assert!(c.is_empty());
}

#[test]
fn add_text_places_text_and_ignores_blank() {
    let mut c = Canvas::new(4.0);
    assert!(!c.add_text(Color(0), pt(1.0, 2.0), "  \n", 0.0));
    assert!(c.add_text(Color(3), pt(1.0, 2.0), "hi", 5.0));
    let s = &c.shapes()[0];
    assert_eq!((s.tool, s.color, s.text.as_deref()), (Tool::Text, Color(3), Some("hi")));
    assert_eq!(s.points, vec![pt(1.0, 2.0)]);
    assert!(close(s.at, 5.0));
}

#[test]
fn undo_and_redo_walk_the_stacks() {
    let mut c = Canvas::new(4.0);
    assert!(!c.undo() && !c.redo());
    drawn(&mut c, Tool::Rect, 0.0);
    drawn(&mut c, Tool::Arrow, 1.0);
    assert!(c.undo());
    assert_eq!(c.shapes().len(), 1);
    assert!(c.undo());
    assert!(!c.undo());
    assert!(c.redo());
    assert_eq!(c.shapes()[0].tool, Tool::Rect);
    assert!(c.redo());
    assert!(!c.redo());
    assert_eq!(c.shapes()[1].tool, Tool::Arrow);
}

#[test]
fn a_new_shape_clears_redo() {
    let mut c = Canvas::new(4.0);
    drawn(&mut c, Tool::Rect, 0.0);
    c.undo();
    drawn(&mut c, Tool::Arrow, 1.0);
    assert!(!c.redo());
    c.undo();
    c.add_text(Color(0), pt(0.0, 0.0), "x", 2.0);
    assert!(!c.redo());
}

#[test]
fn clear_drops_everything() {
    let mut c = Canvas::new(4.0);
    assert!(!c.clear());
    drawn(&mut c, Tool::Rect, 0.0);
    drawn(&mut c, Tool::Rect, 0.0);
    c.undo();
    c.begin(Tool::Pen, Color(0), pt(0.0, 0.0), 0.0);
    assert!(c.clear());
    assert!(c.is_empty());
    assert!(!c.redo());
    c.begin(Tool::Pen, Color(0), pt(0.0, 0.0), 0.0);
    assert!(c.clear(), "an active drag alone is visible");
}

#[test]
fn expire_drops_old_shapes_only_when_fading() {
    let mut c = Canvas::new(4.0);
    drawn(&mut c, Tool::Rect, 0.0);
    drawn(&mut c, Tool::Rect, 2.0);
    drawn(&mut c, Tool::Rect, 4.0);
    c.undo();
    assert!(!c.expire(100.0, 0.0));
    assert!(!c.expire(100.0, -1.0));
    assert!(!c.expire(2.9, 3.0));
    assert!(c.expire(3.0, 3.0));
    assert_eq!(c.shapes().len(), 1);
    assert!(close(c.shapes()[0].at, 2.0));
    assert!(c.redo(), "the undone shape is still fresh");
    assert!(c.expire(10.0, 3.0));
    assert!(c.shapes().is_empty());
    assert!(!c.redo(), "expired undone shapes go too");
    assert!(!c.expire(10.0, 3.0));
}

#[test]
fn arrow_head_points_back_along_the_shaft() {
    let [l, tip, r] = arrow_head(pt(0.0, 0.0), pt(100.0, 0.0), 4.0);
    assert!(near(tip, pt(100.0, 0.0)));
    assert!(near(l, pt(84.0, 8.0)) && near(r, pt(84.0, -8.0)), "{l:?} {r:?}");
    let [l, tip, r] = arrow_head(pt(0.0, 0.0), pt(0.0, 50.0), 1.0);
    assert!(near(tip, pt(0.0, 50.0)));
    assert!(near(l, pt(-5.0, 40.0)) && near(r, pt(5.0, 40.0)), "minimum 10 pt: {l:?} {r:?}");
}

#[test]
fn zero_length_arrow_head_points_right() {
    let [l, tip, r] = arrow_head(pt(5.0, 5.0), pt(5.0, 5.0), 2.0);
    assert!(near(tip, pt(5.0, 5.0)));
    assert!(near(l, pt(-5.0, 10.0)) && near(r, pt(-5.0, 0.0)), "{l:?} {r:?}");
}

#[test]
fn normalized_orders_corners() {
    let want = Rect { x: 2.0, y: 3.0, w: 8.0, h: 4.0 };
    assert_eq!(normalized(pt(2.0, 3.0), pt(10.0, 7.0)), want);
    assert_eq!(normalized(pt(10.0, 7.0), pt(2.0, 3.0)), want);
    assert_eq!(normalized(pt(10.0, 3.0), pt(2.0, 7.0)), want);
}

#[test]
fn scaled_multiplies_points_and_width() {
    let mut c = Canvas::new(3.0);
    c.add_text(Color(2), pt(1.5, 2.0), "t", 9.0);
    let s = scaled(&c.shapes()[0], 2.0);
    assert_eq!(s.points, vec![pt(3.0, 4.0)]);
    assert!(close(f64::from(s.width), 6.0));
    assert_eq!((s.tool, s.color, s.text.as_deref()), (Tool::Text, Color(2), Some("t")));
    assert!(close(s.at, 9.0));
}

#[test]
fn keys_pick_tools() {
    let tools = [
        ("a", Tool::Arrow),
        ("r", Tool::Rect),
        ("p", Tool::Pen),
        ("h", Tool::Highlight),
        ("t", Tool::Text),
        ("x", Tool::Redact),
        ("X", Tool::Redact),
    ];
    for (key, tool) in tools {
        assert_eq!(key_command(key, false, false), Some(Command::Tool(tool)), "{key}");
    }
}

#[test]
fn keys_pick_colors() {
    for (i, key) in ["1", "2", "3", "4", "5"].into_iter().enumerate() {
        let want = Some(Command::Color(Color(i as u8)));
        assert_eq!(key_command(key, false, false), want, "{key}");
    }
    assert_eq!(key_command("6", false, false), None);
}

#[test]
fn keys_without_cmd() {
    let keys = [
        ("\u{7f}", Some(Command::Clear)),
        ("\u{8}", Some(Command::Clear)),
        ("\u{f728}", Some(Command::Clear)),
        ("\r", Some(Command::Done)),
        ("\u{3}", Some(Command::Done)),
        ("\u{1b}", Some(Command::Cancel)),
        ("?", Some(Command::Help)),
        ("/", Some(Command::Help)),
        ("z", None),
        ("", None),
    ];
    for (key, want) in keys {
        assert_eq!(key_command(key, false, false), want, "{key:?}");
    }
}

#[test]
fn keys_with_cmd() {
    let keys = [
        ("z", false, Some(Command::Undo)),
        ("z", true, Some(Command::Redo)),
        ("Z", true, Some(Command::Redo)),
        ("c", false, Some(Command::Copy)),
        ("s", false, Some(Command::Save)),
        ("a", false, None),
        ("\r", false, None),
    ];
    for (key, shift, want) in keys {
        assert_eq!(key_command(key, true, shift), want, "{key:?} shift={shift}");
    }
}
