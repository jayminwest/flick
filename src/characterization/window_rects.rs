//! Window rect math in the Accessibility coordinate space (origin top-left, y down).

use crate::modules::windows::action::{Rect, WindowAction, frame_for};

const AREA: Rect = Rect { x: 0.0, y: 25.0, w: 1200.0, h: 800.0 };
const WIN: Rect = Rect { x: 100.0, y: 100.0, w: 400.0, h: 300.0 };

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn every_action_target() {
    let expected = [
        (WindowAction::LeftHalf, rect(0.0, 25.0, 600.0, 800.0)),
        (WindowAction::RightHalf, rect(600.0, 25.0, 600.0, 800.0)),
        (WindowAction::TopHalf, rect(0.0, 25.0, 1200.0, 400.0)),
        (WindowAction::BottomHalf, rect(0.0, 425.0, 1200.0, 400.0)),
        (WindowAction::TopLeft, rect(0.0, 25.0, 600.0, 400.0)),
        (WindowAction::TopRight, rect(600.0, 25.0, 600.0, 400.0)),
        (WindowAction::BottomLeft, rect(0.0, 425.0, 600.0, 400.0)),
        (WindowAction::BottomRight, rect(600.0, 425.0, 600.0, 400.0)),
        (WindowAction::FirstThird, rect(0.0, 25.0, 400.0, 800.0)),
        (WindowAction::CenterThird, rect(400.0, 25.0, 400.0, 800.0)),
        (WindowAction::LastThird, rect(800.0, 25.0, 400.0, 800.0)),
        (WindowAction::FirstTwoThirds, rect(0.0, 25.0, 800.0, 800.0)),
        (WindowAction::LastTwoThirds, rect(400.0, 25.0, 800.0, 800.0)),
        (WindowAction::Maximize, AREA),
        (WindowAction::AlmostMaximize, rect(60.0, 65.0, 1080.0, 720.0)),
        (WindowAction::Center, rect(400.0, 275.0, 400.0, 300.0)),
        (WindowAction::NextDisplay, WIN),
        (WindowAction::PreviousDisplay, WIN),
        (WindowAction::Minimize, WIN),
        (WindowAction::Hide, WIN),
    ];
    assert_eq!(expected.len(), WindowAction::ALL.len());
    for (action, want) in expected {
        assert_eq!(action.target(AREA, WIN), want, "{action:?}");
    }
}

#[test]
fn center_clamps_to_the_area() {
    let big = rect(0.0, 0.0, 5000.0, 5000.0);
    assert_eq!(WindowAction::Center.target(AREA, big), AREA);
}

#[test]
fn repeated_halves_cycle_half_two_thirds_third() {
    let cycles = [
        (WindowAction::LeftHalf, [600.0, 800.0, 400.0], true),
        (WindowAction::RightHalf, [600.0, 800.0, 400.0], true),
        (WindowAction::TopHalf, [400.0, 533.0, 267.0], false),
        (WindowAction::BottomHalf, [400.0, 533.0, 267.0], false),
    ];
    for (action, sizes, horizontal) in cycles {
        let mut frame = WIN;
        for size in sizes.into_iter().chain([sizes[0]]) {
            frame = action.cycled(AREA, frame);
            let got = if horizontal { frame.w } else { frame.h };
            assert_eq!(got, size, "{action:?}");
        }
    }
    let right = WindowAction::RightHalf;
    assert_eq!(right.cycled(AREA, WIN), rect(600.0, 25.0, 600.0, 800.0));
    let two_thirds = right.cycled(AREA, right.cycled(AREA, WIN));
    assert_eq!(two_thirds, rect(400.0, 25.0, 800.0, 800.0));
    // Frames within 4 points count as the same size, since apps round their frames.
    assert_eq!(
        right.cycled(AREA, Rect { w: 799.0, ..two_thirds }),
        rect(800.0, 25.0, 400.0, 800.0)
    );
    let bottom = WindowAction::BottomHalf;
    assert_eq!(bottom.cycled(AREA, bottom.cycled(AREA, WIN)), rect(0.0, 292.0, 1200.0, 533.0));
}

#[test]
fn non_halves_do_not_cycle() {
    for action in WindowAction::ALL {
        if matches!(
            action,
            WindowAction::LeftHalf
                | WindowAction::RightHalf
                | WindowAction::TopHalf
                | WindowAction::BottomHalf
        ) {
            continue;
        }
        let once = action.cycled(AREA, WIN);
        assert_eq!(once, action.target(AREA, WIN), "{action:?}");
        assert_eq!(action.cycled(AREA, once), action.target(AREA, once), "{action:?}");
    }
}

#[test]
fn frame_uses_the_screen_holding_the_window_center() {
    let right_screen = rect(1200.0, 0.0, 2400.0, 1600.0);
    let areas = [AREA, right_screen];
    let on_right = rect(1300.0, 100.0, 400.0, 300.0);
    assert_eq!(
        frame_for(WindowAction::LeftHalf, on_right, &areas),
        Ok(rect(1200.0, 0.0, 1200.0, 1600.0))
    );
    // A center on no screen falls back to the first.
    let offscreen = rect(-5000.0, -5000.0, 400.0, 300.0);
    assert_eq!(frame_for(WindowAction::Maximize, offscreen, &areas), Ok(AREA));
    assert_eq!(frame_for(WindowAction::Maximize, WIN, &[]), Err("No screens"));
}

#[test]
fn display_moves_keep_relative_position_and_wrap() {
    let right_screen = rect(1200.0, 0.0, 2400.0, 1600.0);
    let areas = [AREA, right_screen];
    let left = rect(0.0, 25.0, 400.0, 300.0);
    assert_eq!(
        frame_for(WindowAction::NextDisplay, left, &areas),
        Ok(rect(1200.0, 0.0, 400.0, 300.0))
    );
    // Halfway across the free space stays halfway; size clamps to the smaller screen.
    let mid = rect(1200.0 + 1000.0, 650.0, 400.0, 300.0);
    assert_eq!(
        frame_for(WindowAction::PreviousDisplay, mid, &areas),
        Ok(rect(400.0, 25.0 + 250.0, 400.0, 300.0))
    );
    assert_eq!(
        frame_for(WindowAction::NextDisplay, mid, &areas),
        frame_for(WindowAction::PreviousDisplay, mid, &areas)
    );
    let huge = rect(1200.0, 0.0, 2400.0, 1600.0);
    assert_eq!(frame_for(WindowAction::NextDisplay, huge, &areas), Ok(AREA));
    assert_eq!(frame_for(WindowAction::NextDisplay, WIN, &[AREA]), Ok(WIN));
}
