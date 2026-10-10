//! Pure layout and text of the recording pill: sizes, where it goes on the screen, the
//! frames of its parts, the level bar's fill, and when it hides. No `AppKit`.

/// Pill height; the ends are fully rounded.
pub const H: f64 = 32.0;
/// Gap between the bottom of the visible screen area and the pill.
const BOTTOM: f64 = 96.0;
const PAD: f64 = 14.0;
const GAP: f64 = 8.0;
const DOT: f64 = 8.0;
pub const BAR_W: f64 = 64.0;
const BAR_H: f64 = 4.0;
/// Text line height inside the pill.
const TEXT_H: f64 = 16.0;
const MIN_W: f64 = 140.0;
const MAX_W: f64 = 460.0;
/// Characters of a result or error kept on its one line.
const MAX_CHARS: usize = 90;
/// Seconds a result and an error stay up.
const RESULT_SECS: f64 = 1.5;
const ERROR_SECS: f64 = 4.0;

/// A frame in `AppKit` screen coordinates (origin bottom-left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
}

/// What the pill shows, without its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Recording,
    Transcribing,
    Result,
    Error,
}

impl Kind {
    /// Esc cancels work in progress; on a result or an error it only hides the pill.
    pub fn cancellable(self) -> bool {
        matches!(self, Kind::Recording | Kind::Transcribing)
    }
}

/// The pill's text: fixed for recording and transcribing, else `text` on one line.
pub fn label(kind: Kind, text: &str) -> String {
    match kind {
        Kind::Recording => "Listening".into(),
        Kind::Transcribing => "Transcribing\u{2026}".into(),
        Kind::Result | Kind::Error => one_line(text, MAX_CHARS),
    }
}

/// `text` with every run of whitespace (newlines included) as one space, trimmed, and cut
/// to `max` characters with an ellipsis.
pub fn one_line(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max {
        return joined;
    }
    let mut cut: String = joined.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}

/// The pill's width for a label `text_w` points wide.
pub fn width(kind: Kind, text_w: f64) -> f64 {
    let extra = if kind == Kind::Recording { DOT + GAP + GAP + BAR_W } else { 0.0 };
    (PAD + text_w.max(0.0) + extra + PAD).clamp(MIN_W, MAX_W)
}

/// Where a pill `w` wide goes: centred near the bottom of `area` (a screen's visible frame).
pub fn frame(area: Rect, w: f64) -> Rect {
    Rect::new(area.x + ((area.w - w) / 2.0).round(), area.y + BOTTOM, w, H)
}

/// Frames of the pill's parts inside a pill `w` wide (origin at its bottom-left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts {
    /// The red recording dot.
    pub dot: Option<Rect>,
    pub text: Rect,
    /// The level bar's track; the fill starts at its left edge.
    pub track: Option<Rect>,
}

pub fn parts(kind: Kind, w: f64) -> Parts {
    let text_y = ((H - TEXT_H) / 2.0).round();
    if kind != Kind::Recording {
        return Parts {
            dot: None,
            text: Rect::new(PAD, text_y, w - 2.0 * PAD, TEXT_H),
            track: None,
        };
    }
    let dot = Rect::new(PAD, (H - DOT) / 2.0, DOT, DOT);
    let track = Rect::new(w - PAD - BAR_W, (H - BAR_H) / 2.0, BAR_W, BAR_H);
    let text_x = PAD + DOT + GAP;
    let text = Rect::new(text_x, text_y, (track.x - GAP - text_x).max(0.0), TEXT_H);
    Parts { dot: Some(dot), text, track: Some(track) }
}

/// The fill of a level bar `track` wide for `level` (0..=1; out of range clamps, NaN is 0).
/// Never thinner than the bar is tall, so silence still shows a dot.
pub fn fill(level: f32, track: f64) -> f64 {
    let level = if level.is_nan() { 0.0 } else { f64::from(level).clamp(0.0, 1.0) };
    (track * level).max(BAR_H).min(track)
}

/// Seconds until the pill hides by itself, if it does.
pub fn hide_after(kind: Kind) -> Option<f64> {
    match kind {
        Kind::Result => Some(RESULT_SECS),
        Kind::Error => Some(ERROR_SECS),
        Kind::Recording | Kind::Transcribing => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn labels_are_fixed_or_one_line() {
        assert_eq!(label(Kind::Recording, "ignored"), "Listening");
        assert_eq!(label(Kind::Transcribing, ""), "Transcribing\u{2026}");
        assert_eq!(label(Kind::Result, "  hello\n\tworld  "), "hello world");
        assert_eq!(label(Kind::Error, "engine failed"), "engine failed");
        let long = "word ".repeat(40);
        let cut = label(Kind::Result, &long);
        assert_eq!(cut.chars().count(), MAX_CHARS);
        assert!(cut.ends_with("word\u{2026}"), "{cut}");
    }

    #[test]
    fn one_line_cuts_by_characters() {
        assert_eq!(one_line("abcdef", 6), "abcdef");
        assert_eq!(one_line("abcdefg", 6), "abcde\u{2026}");
        assert_eq!(one_line("\u{e9}\u{e9}\u{e9}\u{e9}", 3), "\u{e9}\u{e9}\u{2026}");
        assert_eq!(one_line("abc", 0), "\u{2026}");
        assert_eq!(one_line("   ", 5), "");
    }

    #[test]
    fn width_fits_the_text_within_bounds() {
        assert!(close(width(Kind::Transcribing, 100.0), MIN_W));
        assert!(close(width(Kind::Result, 200.0), 228.0));
        assert!(close(width(Kind::Recording, 60.0), 2.0 * PAD + 60.0 + DOT + 2.0 * GAP + BAR_W));
        assert!(close(width(Kind::Error, 5000.0), MAX_W));
        assert!(close(width(Kind::Result, -3.0), MIN_W));
    }

    #[test]
    fn frame_centres_near_the_bottom() {
        let area = Rect::new(100.0, 25.0, 1000.0, 800.0);
        assert_eq!(frame(area, 200.0), Rect::new(500.0, 25.0 + BOTTOM, 200.0, H));
        // Whole points, so the pill is never blurry.
        assert!(close(frame(area, 201.0).x, 500.0));
    }

    #[test]
    fn recording_parts_hold_dot_text_and_bar() {
        let w = width(Kind::Recording, 60.0);
        let p = parts(Kind::Recording, w);
        let (dot, track) = (p.dot.unwrap(), p.track.unwrap());
        assert!(close(dot.x, PAD));
        assert!(close(dot.y + dot.h / 2.0, H / 2.0));
        assert!(close(p.text.x, PAD + DOT + GAP));
        assert!(close(p.text.w, 60.0));
        assert!(close(track.x + track.w, w - PAD));
        assert!(close(track.y + track.h / 2.0, H / 2.0));
        assert!(p.text.x + p.text.w <= track.x);
    }

    #[test]
    fn text_parts_span_the_pill() {
        let p = parts(Kind::Error, 300.0);
        assert_eq!(p.dot, None);
        assert_eq!(p.track, None);
        assert_eq!(p.text, Rect::new(PAD, 8.0, 300.0 - 2.0 * PAD, TEXT_H));
        // A recording pill too narrow for its text gets no negative width.
        assert!(close(parts(Kind::Recording, 10.0).text.w, 0.0));
    }

    #[test]
    fn fill_clamps_and_keeps_a_dot() {
        assert!(close(fill(0.5, 64.0), 32.0));
        assert!(close(fill(1.0, 64.0), 64.0));
        assert!(close(fill(7.0, 64.0), 64.0));
        assert!(close(fill(0.0, 64.0), BAR_H));
        assert!(close(fill(-1.0, 64.0), BAR_H));
        assert!(close(fill(f32::NAN, 64.0), BAR_H));
        assert!(close(fill(0.5, 2.0), 2.0));
    }

    #[test]
    fn only_results_and_errors_hide_by_themselves() {
        assert_eq!(hide_after(Kind::Recording), None);
        assert_eq!(hide_after(Kind::Transcribing), None);
        assert_eq!(hide_after(Kind::Result), Some(RESULT_SECS));
        assert_eq!(hide_after(Kind::Error), Some(ERROR_SECS));
        assert!(Kind::Recording.cancellable() && Kind::Transcribing.cancellable());
        assert!(!Kind::Result.cancellable() && !Kind::Error.cancellable());
    }
}
