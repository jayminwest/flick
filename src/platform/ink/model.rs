//! The annotation model: shapes on a canvas, undo and redo, fade-out, arrow-head geometry
//! and the editor key map. Plain Rust with no `AppKit`, so every branch has a unit test; the
//! canvas view feeds it mouse and key events and draws what it holds.
//!
//! Coordinates are points in a flipped view (origin top-left, y grows downward). `scaled`
//! turns a shape into pixels for export. Times are seconds on any monotonic clock the
//! caller picks; the model only compares them.

use crate::platform::Rect;

/// A pen or highlighter point closer than this to the previous one is skipped.
const MIN_STEP: f64 = 1.0;
/// A rectangle, redact box or arrow smaller than this on its axes is a click, not a shape.
const MIN_SIZE: f64 = 1.0;

/// A position in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    fn distance(self, other: Point) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

/// An index into `Style::palette`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color(pub u8);

/// How new shapes look and how long overlay strokes stay.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// RGBA, each 0.0 to 1.0. `Color(n)` picks entry n.
    pub palette: Vec<[f32; 4]>,
    /// Stroke width in points.
    pub width: f32,
    /// Seconds a finished overlay stroke stays; 0 keeps it until cleared.
    pub fade_secs: f32,
}

impl Default for Style {
    /// Red, yellow, green, blue, white; 4 pt strokes; no fade.
    fn default() -> Self {
        Style {
            palette: vec![
                [1.0, 0.231, 0.188, 1.0],
                [1.0, 0.8, 0.0, 1.0],
                [0.204, 0.78, 0.349, 1.0],
                [0.039, 0.518, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
            ],
            width: 4.0,
            fade_secs: 0.0,
        }
    }
}

impl Style {
    /// The RGBA for `color`; an index past the palette (or an empty palette) is opaque red.
    pub fn rgba(&self, color: Color) -> [f32; 4] {
        self.palette.get(usize::from(color.0)).copied().unwrap_or([1.0, 0.0, 0.0, 1.0])
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Arrow,
    Rect,
    Pen,
    /// A pen stroke drawn wide and translucent.
    Highlight,
    Text,
    /// A filled black box over what it covers.
    Redact,
}

impl Tool {
    /// Pen and highlight keep every point; the others keep only start and end.
    fn freehand(self) -> bool {
        matches!(self, Tool::Pen | Tool::Highlight)
    }
}

/// One finished or in-progress mark.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub tool: Tool,
    pub color: Color,
    pub width: f32,
    /// Freehand: the stroke. Arrow: tail then tip. Rect and redact: two opposite corners.
    /// Text: the top-left of the text.
    pub points: Vec<Point>,
    pub text: Option<String>,
    /// When the shape was started, for fading.
    pub at: f64,
}

/// What the canvas shows: finished shapes in drawing order, the redo stack, and the shape
/// under the mouse.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Canvas {
    /// Stroke width for new shapes, in points.
    pub width: f32,
    shapes: Vec<Shape>,
    undone: Vec<Shape>,
    drag: Option<Shape>,
}

impl Canvas {
    pub fn new(width: f32) -> Self {
        Canvas { width, ..Canvas::default() }
    }

    /// Finished shapes, oldest first.
    #[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 6 (flick-2759)"))]
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// The shape being dragged, if any.
    #[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 6 (flick-2759)"))]
    pub fn active(&self) -> Option<&Shape> {
        self.drag.as_ref()
    }

    /// Everything to draw: finished shapes, then the active one on top.
    pub fn visible(&self) -> impl Iterator<Item = &Shape> {
        self.shapes.iter().chain(self.drag.as_ref())
    }

    /// No finished and no active shape.
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty() && self.drag.is_none()
    }

    /// Mouse down: start a shape at `p`. Text is not dragged (the view asks for the string and
    /// calls `add_text`), so it starts nothing. A previous unfinished drag is dropped.
    pub fn begin(&mut self, tool: Tool, color: Color, p: Point, now: f64) {
        self.drag = (tool != Tool::Text).then(|| Shape {
            tool,
            color,
            width: self.width,
            points: vec![p],
            text: None,
            at: now,
        });
    }

    /// Mouse dragged: pen and highlight append `p` once it is more than 1 pt from the last
    /// point; arrow, rect and redact move their end to `p`.
    pub fn drag(&mut self, p: Point) {
        let Some(shape) = &mut self.drag else { return };
        if shape.tool.freehand() {
            if shape.points.last().is_none_or(|last| last.distance(p) > MIN_STEP) {
                shape.points.push(p);
            }
        } else {
            shape.points.truncate(1);
            shape.points.push(p);
        }
    }

    /// Mouse up: keep the active shape and clear the redo stack. A click with the arrow, rect
    /// or redact tool (no size) is dropped. Returns whether a shape was added.
    pub fn end(&mut self) -> bool {
        let Some(shape) = self.drag.take() else { return false };
        if !shape.tool.freehand() && !sized(&shape.points) {
            return false;
        }
        self.push(shape);
        true
    }

    /// Place `text` with its top-left at `p`. Blank text adds nothing.
    pub fn add_text(&mut self, color: Color, p: Point, text: &str, now: f64) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        self.push(Shape {
            tool: Tool::Text,
            color,
            width: self.width,
            points: vec![p],
            text: Some(text.to_owned()),
            at: now,
        });
        true
    }

    fn push(&mut self, shape: Shape) {
        self.shapes.push(shape);
        self.undone.clear();
    }

    /// Take back the newest shape. Returns whether there was one.
    pub fn undo(&mut self) -> bool {
        let Some(shape) = self.shapes.pop() else { return false };
        self.undone.push(shape);
        true
    }

    /// Put back the newest undone shape. Returns whether there was one.
    pub fn redo(&mut self) -> bool {
        let Some(shape) = self.undone.pop() else { return false };
        self.shapes.push(shape);
        true
    }

    /// Drop every shape, the redo stack and the active drag. Returns whether anything was
    /// visible.
    pub fn clear(&mut self) -> bool {
        let visible = !self.is_empty();
        self.shapes.clear();
        self.undone.clear();
        self.drag = None;
        visible
    }

    /// Drop finished shapes (and undone ones) started `fade_secs` or more before `now`. A
    /// `fade_secs` of 0 or less never expires anything. Returns whether a visible shape went.
    #[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 4 (flick-ee1d)"))]
    pub fn expire(&mut self, now: f64, fade_secs: f32) -> bool {
        if fade_secs <= 0.0 {
            return false;
        }
        let live = |s: &Shape| now - s.at < f64::from(fade_secs);
        let before = self.shapes.len();
        self.shapes.retain(live);
        self.undone.retain(live);
        self.shapes.len() != before
    }
}

/// Two points far enough apart to draw: an arrow with length, a box with area.
fn sized(points: &[Point]) -> bool {
    match points {
        [a, b] => (a.x - b.x).abs() >= MIN_SIZE || (a.y - b.y).abs() >= MIN_SIZE,
        _ => false,
    }
}

/// The filled head of an arrow from `from` to `to`: `[left, tip, right]`, with the tip at
/// `to`. It grows with the stroke width (4x, at least 10 pt long) and is as wide as it is
/// long. A zero-length arrow points right.
pub fn arrow_head(from: Point, to: Point, width: f32) -> [Point; 3] {
    let len = f64::from(width * 4.0).max(10.0);
    let d = from.distance(to);
    let (ux, uy) = if d > 0.0 { ((to.x - from.x) / d, (to.y - from.y) / d) } else { (1.0, 0.0) };
    let (bx, by) = (to.x - ux * len, to.y - uy * len);
    let (nx, ny) = (-uy * len / 2.0, ux * len / 2.0);
    [Point::new(bx + nx, by + ny), to, Point::new(bx - nx, by - ny)]
}

/// The rectangle with `a` and `b` as opposite corners, whichever way it was dragged.
pub fn normalized(a: Point, b: Point) -> Rect {
    Rect { x: a.x.min(b.x), y: a.y.min(b.y), w: (a.x - b.x).abs(), h: (a.y - b.y).abs() }
}

/// `shape` with its points and width multiplied by `k`: points to pixels on export.
pub fn scaled(shape: &Shape, k: f64) -> Shape {
    Shape {
        points: shape.points.iter().map(|p| Point::new(p.x * k, p.y * k)).collect(),
        width: (f64::from(shape.width) * k) as f32,
        ..shape.clone()
    }
}

/// What a key does in the editor or the drawing overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Tool(Tool),
    Color(Color),
    Undo,
    Redo,
    Clear,
    /// Return: save (and copy when configured) and close.
    Done,
    /// Cmd+C: save, copy and close.
    Copy,
    /// Cmd+S: save without copying.
    Save,
    /// Esc: close without saving.
    Cancel,
}

/// Map a key press to a command. `chars` is `charactersIgnoringModifiers` (shift may make it
/// upper case); `cmd` and `shift` are the modifier flags. Keys: a r p h t x pick arrow, rect,
/// pen, highlight, text, redact; 1-5 pick a color; Delete clears; Return is done; Esc
/// cancels; cmd+z undo, cmd+shift+z redo, cmd+c copy, cmd+s save.
pub fn key_command(chars: &str, cmd: bool, shift: bool) -> Option<Command> {
    let key = chars.to_ascii_lowercase();
    if cmd {
        return match key.as_str() {
            "z" if shift => Some(Command::Redo),
            "z" => Some(Command::Undo),
            "c" => Some(Command::Copy),
            "s" => Some(Command::Save),
            _ => None,
        };
    }
    Some(match key.as_str() {
        "a" => Command::Tool(Tool::Arrow),
        "r" => Command::Tool(Tool::Rect),
        "p" => Command::Tool(Tool::Pen),
        "h" => Command::Tool(Tool::Highlight),
        "t" => Command::Tool(Tool::Text),
        "x" => Command::Tool(Tool::Redact),
        "1" => Command::Color(Color(0)),
        "2" => Command::Color(Color(1)),
        "3" => Command::Color(Color(2)),
        "4" => Command::Color(Color(3)),
        "5" => Command::Color(Color(4)),
        // Backspace (Delete) and forward delete.
        "\u{7f}" | "\u{8}" | "\u{f728}" => Command::Clear,
        // Return and keypad Enter.
        "\r" | "\u{3}" => Command::Done,
        "\u{1b}" => Command::Cancel,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
