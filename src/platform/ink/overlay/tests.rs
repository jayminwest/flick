use super::*;
use crate::platform::ink::model::{Color, Tool};

fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn return_stops_drawing_and_esc_also_clears() {
    assert_eq!(key_action(Command::Done), Some(Action::Stop));
    assert_eq!(key_action(Command::Cancel), Some(Action::StopAndClear));
    for cmd in [
        Command::Copy,
        Command::Save,
        Command::Tool(Tool::Pen),
        Command::Color(Color(3)),
        Command::Undo,
        Command::Redo,
        Command::Clear,
    ] {
        assert_eq!(key_action(cmd), None);
    }
}

#[test]
fn relayout_keeps_displays_whose_frame_is_unchanged() {
    let main = r(0.0, 0.0, 1440.0, 900.0);
    let side = r(1440.0, 0.0, 1920.0, 1080.0);
    let moved = r(-1920.0, 0.0, 1920.0, 1080.0);
    assert_eq!(matches(&[main, side], &[main, side]), [Some(0), Some(1)]);
    assert_eq!(matches(&[main, side], &[side]), [Some(1)]);
    assert_eq!(matches(&[main, side], &[main, moved]), [Some(0), None]);
    assert_eq!(matches(&[], &[main]), [None]);
    // Two displays with one frame (mirroring) never share a panel.
    assert_eq!(matches(&[main], &[main, main]), [Some(0), None]);
}

#[test]
fn the_halo_is_centered_on_the_pointer() {
    let f = halo_frame(NSPoint::new(100.0, 200.0), 28.0);
    assert_eq!(f, r(69.0, 169.0, 62.0, 62.0));
    // A zero radius still draws a small ring.
    assert!((halo_side(0.0) - 8.0).abs() < 1e-9);
}

#[test]
fn points_on_a_right_or_top_edge_belong_to_the_next_display() {
    let main = r(0.0, 0.0, 1440.0, 900.0);
    assert!(contains(main, NSPoint::new(0.0, 0.0)));
    assert!(contains(main, NSPoint::new(1439.5, 899.5)));
    assert!(!contains(main, NSPoint::new(1440.0, 10.0)));
    assert!(!contains(main, NSPoint::new(10.0, 900.0)));
    assert!(!contains(main, NSPoint::new(-1.0, 10.0)));
}

#[test]
fn frames_convert_both_ways() {
    let f = r(-1920.0, 120.0, 1920.0, 1080.0);
    assert_eq!(rect(ns_rect(f)), f);
}
