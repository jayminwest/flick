//! The key legend: what the strip at the bottom of the draw overlay and the annotation
//! editor says (the current tool and color, and the keys), and where it goes. Plain Rust
//! with a 100% coverage floor; `hud` draws it. The `help` module lists the same keys, so
//! this is their one source.

use super::model::{Color, Style, Tool};
use crate::platform::Rect;

/// Room between the strip and the view's bottom edge, and inside the strip.
pub const MARGIN: f64 = 24.0;
pub const PAD: f64 = 10.0;
/// Each color swatch's side, and the gap after it.
pub const SWATCH: f64 = 14.0;
pub const GAP: f64 = 6.0;

/// Which keys the strip lists: the editor's Return saves (and copies when `copy`), the
/// overlay's stops drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Editor { copy: bool },
    Overlay,
}

/// One key: what to press, a word or two for the strip, a sentence for the help view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub press: &'static str,
    pub short: &'static str,
    pub long: &'static str,
}

const fn key(press: &'static str, short: &'static str, long: &'static str) -> Key {
    Key { press, short, long }
}

/// The tool keys, in strip order.
pub const TOOLS: [(Key, Tool); 6] = [
    (key("A", "arrow", "Arrow: drag from tail to tip"), Tool::Arrow),
    (key("R", "rect", "Rectangle: drag a box outline"), Tool::Rect),
    (key("P", "pen", "Pen: draw freehand"), Tool::Pen),
    (key("H", "highlight", "Highlighter: a wide, see-through pen"), Tool::Highlight),
    (key("T", "text", "Text: click, type, then Return places it"), Tool::Text),
    (key("X", "redact", "Redact: drag a solid black box"), Tool::Redact),
];

const COLORS: Key =
    key("1-5", "color", "Pick color 1 to 5 (by default red, yellow, green, blue, white)");
const UNDO: Key = key("⌘Z", "undo", "Undo the last shape (there is no eraser)");
const REDO: Key = key("⇧⌘Z", "redo", "Redo");
const CLEAR: Key = key("⌫", "clear all", "Delete clears every shape");
const HELP: Key = key("?", "hide keys", "Show or hide this key legend");

/// Every key in `mode`, in strip order: tools, colors, editing, then how to finish.
pub fn keys(mode: Mode) -> Vec<Key> {
    let finish: &[Key] = match mode {
        Mode::Overlay => &[
            key("↩", "done", "Return stops drawing; the shapes stay, clicks pass through"),
            key("esc", "stop and clear", "Esc stops drawing and clears the shapes"),
        ],
        Mode::Editor { copy: true } => &[
            key("↩", "save and copy", "Return saves the PNG and copies it"),
            key("⌘S", "save", "Save without copying"),
            key("esc", "cancel", "Esc closes without saving"),
        ],
        Mode::Editor { copy: false } => &[
            key("↩", "save", "Return saves the PNG"),
            key("⌘C", "save and copy", "Save and copy the PNG"),
            key("esc", "cancel", "Esc closes without saving"),
        ],
    };
    TOOLS
        .iter()
        .map(|(k, _)| *k)
        .chain([COLORS, UNDO, REDO, CLEAR])
        .chain(finish.iter().copied())
        .chain([HELP])
        .collect()
}

/// The tool's name, as the strip shows it.
pub fn tool_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Arrow => "Arrow",
        Tool::Rect => "Rectangle",
        Tool::Pen => "Pen",
        Tool::Highlight => "Highlighter",
        Tool::Text => "Text",
        Tool::Redact => "Redact",
    }
}

/// Named colors (Apple's system palette) to name a configured RGBA by.
const NAMES: [(&str, [f32; 3]); 11] = [
    ("Red", [1.0, 0.231, 0.188]),
    ("Orange", [1.0, 0.584, 0.0]),
    ("Yellow", [1.0, 0.8, 0.0]),
    ("Green", [0.204, 0.78, 0.349]),
    ("Teal", [0.188, 0.69, 0.78]),
    ("Blue", [0.039, 0.518, 1.0]),
    ("Purple", [0.686, 0.322, 0.871]),
    ("Pink", [1.0, 0.176, 0.333]),
    ("White", [1.0, 1.0, 1.0]),
    ("Gray", [0.557, 0.557, 0.576]),
    ("Black", [0.0, 0.0, 0.0]),
];

/// The name of the closest named color to `rgba` (alpha ignored).
pub fn color_name([r, g, b, _]: [f32; 4]) -> &'static str {
    let dist = |c: &[f32; 3]| (c[0] - r).powi(2) + (c[1] - g).powi(2) + (c[2] - b).powi(2);
    NAMES.iter().min_by(|a, b| dist(&a.1).total_cmp(&dist(&b.1))).map_or("Red", |(name, _)| name)
}

/// The strip's text: the current tool and color, the tool keys, then the other keys.
pub fn lines(mode: Mode, tool: Tool, color: Color, style: &Style) -> [String; 3] {
    let n = color.0 + 1;
    let status = format!("{}  ·  {} ({n})", tool_name(tool), color_name(style.rgba(color)));
    let join = |keys: &[Key]| {
        keys.iter().map(|k| format!("{} {}", k.press, k.short)).collect::<Vec<_>>().join("   ")
    };
    let all = keys(mode);
    let (tools, rest) = all.split_at(TOOLS.len());
    [status, join(tools), join(rest)]
}

/// The strip's frame in a flipped view of `view_w` x `view_h`: centered at the bottom, wide
/// enough for `text_w` (the widest line) and the swatch row, `text_h` high plus padding.
/// Never wider than the view.
pub fn frame(view_w: f64, view_h: f64, text_w: f64, text_h: f64, swatches: usize) -> Rect {
    let w = (text_w.max(swatch_row(swatches)) + 2.0 * PAD).min(view_w);
    let h = text_h + SWATCH + GAP + 2.0 * PAD;
    let y = (view_h - MARGIN - h).max(0.0);
    Rect { x: ((view_w - w) / 2.0).max(0.0), y, w, h }
}

/// The swatches' frames, in a row at the top left inside `strip`.
pub fn swatches(strip: Rect, n: usize) -> Vec<Rect> {
    (0..n)
        .map(|i| Rect {
            x: strip.x + PAD + i as f64 * (SWATCH + GAP),
            y: strip.y + PAD,
            w: SWATCH,
            h: SWATCH,
        })
        .collect()
}

/// Width of a row of `n` swatches.
fn swatch_row(n: usize) -> f64 {
    if n == 0 { 0.0 } else { n as f64 * (SWATCH + GAP) - GAP }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_lists_tools_colors_and_how_to_finish() {
        let overlay: Vec<&str> = keys(Mode::Overlay).iter().map(|k| k.press).collect();
        assert_eq!(
            overlay,
            ["A", "R", "P", "H", "T", "X", "1-5", "⌘Z", "⇧⌘Z", "⌫", "↩", "esc", "?"]
        );
        let copy: Vec<&str> = keys(Mode::Editor { copy: true }).iter().map(|k| k.press).collect();
        assert!(copy.ends_with(&["↩", "⌘S", "esc", "?"]));
        let plain = keys(Mode::Editor { copy: false });
        assert!(plain.iter().any(|k| k.press == "⌘C" && k.short == "save and copy"));
        assert!(plain.iter().all(|k| !k.long.is_empty()));
    }

    #[test]
    fn tool_keys_match_the_editor_key_map() {
        use super::super::model::{Command, key_command};
        for (k, tool) in TOOLS {
            assert_eq!(
                key_command(k.press, false, false),
                Some(Command::Tool(tool)),
                "{}",
                k.press
            );
            assert!(!tool_name(tool).is_empty());
        }
        assert_eq!(key_command(HELP.press, false, false), Some(Command::Help));
    }

    #[test]
    fn colors_are_named_by_the_closest_system_color() {
        let style = Style::default();
        let names: Vec<&str> = (0..5).map(|i| color_name(style.rgba(Color(i)))).collect();
        assert_eq!(names, ["Red", "Yellow", "Green", "Blue", "White"]);
        assert_eq!(color_name([0.1, 0.05, 0.0, 1.0]), "Black");
        assert_eq!(color_name([0.95, 0.5, 0.05, 0.5]), "Orange");
    }

    #[test]
    fn the_strip_shows_tool_color_and_keys() {
        let style = Style::default();
        let [status, tools, rest] = lines(Mode::Overlay, Tool::Highlight, Color(3), &style);
        assert_eq!(status, "Highlighter  ·  Blue (4)");
        assert!(tools.starts_with("A arrow   R rect   P pen"), "{tools}");
        assert!(rest.starts_with("1-5 color   ⌘Z undo"), "{rest}");
        assert!(rest.ends_with("esc stop and clear   ? hide keys"), "{rest}");
        let [status, ..] = lines(Mode::Editor { copy: true }, Tool::Arrow, Color(9), &style);
        assert_eq!(status, "Arrow  ·  Red (10)");
    }

    #[test]
    fn the_strip_sits_centered_at_the_bottom() {
        let r = frame(1000.0, 800.0, 400.0, 30.0, 5);
        let h = 30.0 + SWATCH + GAP + 2.0 * PAD;
        assert_eq!(r, Rect { x: 290.0, y: 800.0 - MARGIN - h, w: 420.0, h });
        // Never wider than the view, never above its top.
        assert_eq!(frame(100.0, 10.0, 400.0, 30.0, 5), Rect { x: 0.0, y: 0.0, w: 100.0, h });
        // A wide swatch row sets the width; none adds nothing.
        let y = 800.0 - MARGIN - h;
        assert_eq!(frame(1000.0, 800.0, 10.0, 30.0, 20), Rect { x: 293.0, y, w: 414.0, h });
        assert_eq!(frame(1000.0, 800.0, 10.0, 30.0, 0), Rect { x: 485.0, y, w: 30.0, h });
    }

    #[test]
    fn swatches_run_left_to_right_inside_the_padding() {
        let strip = Rect { x: 100.0, y: 50.0, w: 300.0, h: 80.0 };
        let s = swatches(strip, 3);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], Rect { x: 110.0, y: 60.0, w: SWATCH, h: SWATCH });
        assert_eq!(s[2], Rect { x: 150.0, y: 60.0, w: SWATCH, h: SWATCH });
    }
}
