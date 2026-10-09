use super::*;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn shape(tool: Tool, width: f32) -> Shape {
    Shape { tool, color: Color(1), width, points: vec![], text: None, at: 0.0 }
}

#[test]
fn highlight_is_wide_and_translucent() {
    let style = Style::default();
    let (rgba, width) = paint(&shape(Tool::Highlight, 4.0), &style);
    assert!(near(width, 12.0));
    let [r, g, b, a] = style.palette[1];
    let same = |x: [f32; 4], y: [f32; 4]| x.iter().zip(y).all(|(x, y)| (x - y).abs() < 1e-6);
    assert!(same(rgba, [r, g, b, a * 0.35]), "{rgba:?}");
    let (rgba, width) = paint(&shape(Tool::Pen, 4.0), &style);
    assert!(near(width, 4.0));
    assert!(same(rgba, style.palette[1]));
}

#[test]
fn arrow_shafts_stop_at_the_head_and_short_arrows_have_none() {
    let from = Point::new(0.0, 0.0);
    let head = arrow_head(from, Point::new(100.0, 0.0), 4.0);
    let end = shaft_end(from, head).map(|p| (p.x, p.y));
    assert!(end.is_some_and(|(x, y)| near(x, 84.0) && near(y, 0.0)), "{end:?}");
    // Shorter than the 16 pt head: the head alone.
    assert_eq!(shaft_end(from, arrow_head(from, Point::new(10.0, 0.0), 4.0)), None);
}

#[test]
fn text_scales_with_stroke_width() {
    assert!(near(text_size(4.0), 18.0));
    assert!(near(text_size(8.0), 36.0));
}

#[test]
fn the_text_field_runs_to_the_right_edge() {
    let r = field_frame(Point::new(100.0, 50.0), 18.0, 600.0);
    assert!(near(r.x, 98.0) && near(r.y, 48.0) && near(r.w, 502.0) && near(r.h, 26.0));
    // Near the right edge it keeps a usable width.
    assert!(near(field_frame(Point::new(590.0, 0.0), 18.0, 600.0).w, 80.0));
}

#[test]
fn export_scale_maps_view_points_to_pixels() {
    assert!(near(export_scale(2880, 1440.0), 2.0));
    assert!(near(export_scale(1000, 500.0), 2.0));
    assert!(near(export_scale(640, 0.0), 1.0));
    // A shape drawn in a fitted (shrunken) view lands on the right pixels.
    let mut s = shape(Tool::Rect, 4.0);
    s.points = vec![Point::new(10.0, 20.0), Point::new(30.0, 40.0)];
    let px = scaled(&s, export_scale(3000, 1000.0));
    assert_eq!(px.points, [Point::new(30.0, 60.0), Point::new(90.0, 120.0)]);
    assert!((px.width - 12.0).abs() < 1e-6);
}
