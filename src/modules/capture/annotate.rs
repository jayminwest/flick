//! Annotation and drawing on the screen through `platform::ink`: Capture Area and Annotate,
//! the Annotate action and verb (`capture:area-annotate`), Draw on Screen (`capture:draw`),
//! Highlight Cursor (`capture:cursor`) and Clear Drawing (`capture:clear`, only while shapes
//! are on the screen).
//!
//! The editor reports how it closed from inside an `AppKit` callback, so `edited` only
//! stashes the result and posts `ModuleChanged` (mx-fcbc43); `Capture::edited` records a
//! saved copy when the module next handles that event.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use crate::core::{Event, Icon, Item, ItemId};
use crate::platform::capture::png_size;
use crate::platform::events;
use crate::platform::ink::editor::{self, Done, EditOpts};
use crate::platform::ink::model::Style;
use crate::platform::ink::overlay::{self, Halo};
use crate::store::{Store, now};

use super::store::{Row, Shots};
use super::{Capture, Settings, name};

/// The default palette: red, yellow, green, blue, white (keys 1 to 5).
pub const COLORS: [&str; 5] = ["#ff3b30", "#ffcc00", "#34c759", "#0a84ff", "#ffffff"];
/// The default halo color.
pub const HALO: &str = "#ffcc00";

/// `editor::edit`: source, destination, options, `on_done`.
type Edit = fn(&Path, &Path, EditOpts, fn(Done)) -> bool;

/// The editor and overlay calls. Tests get fakes that never open a window, install a
/// monitor or touch the pasteboard.
#[derive(Clone, Copy)]
pub struct InkEnv {
    pub edit: Edit,
    pub set_drawing: fn(bool, &Style),
    pub drawing: fn() -> bool,
    pub has_shapes: fn() -> bool,
    pub clear: fn(),
    pub set_cursor: fn(Option<Halo>),
    pub cursor: fn() -> bool,
    pub relayout: fn(),
}

pub const INK_ENV: InkEnv = InkEnv {
    edit: if cfg!(test) { |_, _, _, _| false } else { editor::edit },
    set_drawing: if cfg!(test) { |_, _| {} } else { overlay::set_drawing },
    drawing: if cfg!(test) { || false } else { overlay::drawing },
    has_shapes: if cfg!(test) { || false } else { overlay::has_shapes },
    clear: if cfg!(test) { || {} } else { overlay::clear },
    set_cursor: if cfg!(test) { |_| {} } else { overlay::set_cursor },
    cursor: if cfg!(test) { || false } else { overlay::cursor },
    relayout: if cfg!(test) { || {} } else { overlay::relayout },
};

/// How shapes and the halo look, from `[capture]`.
pub struct Ink {
    pub style: Style,
    pub halo: Halo,
    /// The open editor's output, and whether to record it as a new row when saved.
    pending: Option<(PathBuf, bool)>,
}

impl Default for Ink {
    fn default() -> Self {
        let color = rgba(HALO).unwrap_or([1.0, 0.8, 0.0, 1.0]);
        Ink { style: Style::default(), halo: Halo { color, radius: 28.0 }, pending: None }
    }
}

impl Ink {
    /// The colors, width, fade and halo of `s`. Errors start with `[capture]: `.
    pub fn new(s: &Settings) -> Result<Ink, String> {
        if s.colors.is_empty() || s.colors.len() > COLORS.len() {
            return Err(format!("[capture]: colors needs 1 to {} entries", COLORS.len()));
        }
        let palette = s.colors.iter().map(|c| rgba(c)).collect::<Result<_, _>>()?;
        if !(s.width > 0.0 && s.width.is_finite()) {
            return Err("[capture]: width must be above 0".into());
        }
        if !(s.fade_secs >= 0.0 && s.fade_secs.is_finite()) {
            return Err("[capture]: fade_secs must be 0 or more".into());
        }
        if !(s.halo_radius > 0.0 && s.halo_radius.is_finite()) {
            return Err("[capture]: halo_radius must be above 0".into());
        }
        let style = Style { palette, width: s.width, fade_secs: s.fade_secs };
        let halo = Halo { color: rgba(&s.halo_color)?, radius: s.halo_radius };
        Ok(Ink { style, halo, pending: None })
    }
}

/// `#rrggbb` or `#rrggbbaa` as RGBA from 0.0 to 1.0.
pub fn rgba(text: &str) -> Result<[f32; 4], String> {
    let bad = || format!("[capture]: bad color {text:?} (want #rrggbb or #rrggbbaa)");
    let hex = text
        .strip_prefix('#')
        .filter(|h| matches!(h.len(), 6 | 8) && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(bad)?;
    let byte = |i: usize| {
        u8::from_str_radix(hex.get(i..i + 2).unwrap_or(""), 16).map(|b| f32::from(b) / 255.0)
    };
    let alpha = if hex.len() == 8 { byte(6) } else { Ok(1.0) };
    match (byte(0), byte(2), byte(4), alpha) {
        (Ok(r), Ok(g), Ok(b), Ok(a)) => Ok([r, g, b, a]),
        _ => Err(bad()),
    }
}

thread_local! {
    /// How the editor closed, until the module handles the `ModuleChanged` it posted.
    static EDITED: Cell<Option<Done>> = const { Cell::new(None) };
}

/// The editor's `on_done`: runs while `AppKit` is mid-event, so it only stashes and posts.
fn edited(done: Done) {
    EDITED.set(Some(done));
    if !cfg!(test) {
        events::post(Event::ModuleChanged { module: "capture" });
    }
}

/// Where Annotate saves a copy of `src`: `<stem> annotated.png` beside it, made unique.
pub fn annotated(src: &Path) -> PathBuf {
    let stem = src.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let dir = src.parent().unwrap_or_else(|| Path::new(""));
    name::unique(dir, &format!("{stem} annotated.png"), Path::exists)
}

/// A root item of this module.
fn item(key: &'static str, title: &str, symbol: &'static str, subtitle: &str) -> Item {
    Item {
        subtitle: subtitle.into(),
        accessory: "Capture".into(),
        keywords: vec!["annotate draw presentify highlight cursor screenshot".into()],
        ..Item::new(ItemId::new("capture", key), title, "Run", Icon::Symbol(symbol))
    }
}

impl Capture {
    /// Capture Area and Annotate, Draw on Screen, Highlight Cursor and (with shapes on the
    /// screen) Clear Drawing; the titles follow the overlay's state.
    pub(super) fn ink_items(&self) -> Vec<Item> {
        let ink = self.env.ink;
        let mut items = vec![
            item(
                "area-annotate",
                "Capture Area and Annotate",
                "pencil.and.outline",
                "Drag to select, then mark it up",
            ),
            if (ink.drawing)() {
                item("draw", "Stop Drawing on Screen", "scribble", "Shapes stay until cleared")
            } else {
                item("draw", "Draw on Screen", "scribble", "Esc stops and clears")
            },
            if (ink.cursor)() {
                item("cursor", "Stop Highlighting Cursor", "cursorarrow.rays", "Hide the halo")
            } else {
                item("cursor", "Highlight Cursor", "cursorarrow.rays", "A ring follows the pointer")
            },
        ];
        if (ink.has_shapes)() {
            items.push(item("clear", "Clear Drawing", "eraser", "Remove shapes from the screen"));
        }
        items
    }

    /// Open the editor on `src`. It saves over `src` (`dest` None) or to `dest`, which is
    /// recorded as a new row once saved. `copy`: Return copies the result too.
    pub(super) fn edit(&mut self, src: &Path, dest: Option<PathBuf>, copy: bool) -> Result<(), String> {
        let record = dest.is_some();
        let dest = dest.unwrap_or_else(|| src.to_path_buf());
        let opts = EditOpts { style: self.ink.style.clone(), copy };
        if !(self.env.ink.edit)(src, &dest, opts, edited) {
            return Err(format!("capture: cannot annotate {} (not an image, or an editor is open)", src.display()));
        }
        self.ink.pending = Some((dest, record));
        Ok(())
    }

    /// After the editor closed: record its copy when it saved one. True when a row was added.
    pub(super) fn edited(&mut self, store: &Store) -> bool {
        let Some(done) = EDITED.take() else { return false };
        let Some((dest, true)) = self.ink.pending.take() else { return false };
        let size = std::fs::read(&dest).ok().and_then(|b| png_size(&b));
        let (Some((width, height)), Done::Saved | Done::Copied) = (size, done) else {
            return false;
        };
        let path = dest.display().to_string();
        let row = Row { id: 0, path, kind: "annotated".into(), width, height, taken: now() };
        store.add_shot(&row, self.settings.history).is_some()
    }

    pub(super) fn set_drawing(&self, on: bool) {
        (self.env.ink.set_drawing)(on, &self.ink.style);
    }

    pub(super) fn set_cursor(&self, on: bool) {
        (self.env.ink.set_cursor)(on.then_some(self.ink.halo));
    }

    /// `draw on|off|toggle|clear`.
    pub(super) fn draw_verb(&self, arg: &str) -> Result<String, String> {
        let on = match arg {
            "on" => true,
            "off" => false,
            "toggle" => !(self.env.ink.drawing)(),
            "clear" => {
                (self.env.ink.clear)();
                return Ok("Cleared the drawing".into());
            }
            _ => return Err("capture: usage: capture draw on|off|toggle|clear".into()),
        };
        self.set_drawing(on);
        Ok(if on { "Drawing on the screen" } else { "Stopped drawing" }.into())
    }

    /// `cursor on|off|toggle`.
    pub(super) fn cursor_verb(&self, arg: &str) -> Result<String, String> {
        let on = match arg {
            "on" => true,
            "off" => false,
            "toggle" => !(self.env.ink.cursor)(),
            _ => return Err("capture: usage: capture cursor on|off|toggle".into()),
        };
        self.set_cursor(on);
        Ok(if on { "Highlighting the cursor" } else { "Stopped highlighting the cursor" }.into())
    }

    /// `annotate <path>`: edit a copy beside the file; answers the copy's path.
    pub(super) fn annotate_verb(&mut self, path: &str) -> Result<String, String> {
        let src = name::expand(path, dirs::home_dir().as_deref());
        if !src.is_file() {
            return Err(format!("capture: no file at {}", src.display()));
        }
        let dest = annotated(&src);
        self.edit(&src, Some(dest.clone()), self.settings.copy)?;
        Ok(dest.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_parse_with_optional_alpha() {
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(near(rgba("#ff0000").unwrap(), [1.0, 0.0, 0.0, 1.0]));
        assert!(near(rgba("#00FF0000").unwrap(), [0.0, 1.0, 0.0, 0.0]));
        for bad in ["ff0000", "#ff00", "#gg0000", "#ff00000", "#é0000"] {
            assert!(rgba(bad).unwrap_err().starts_with("[capture]: bad color"), "{bad}");
        }
    }

    #[test]
    fn annotated_copies_sit_beside_the_source() {
        let dir = std::env::temp_dir().join(format!("flk-{}-annotated", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("Shot.png");
        assert_eq!(annotated(&src), dir.join("Shot annotated.png"));
        std::fs::write(dir.join("Shot annotated.png"), b"x").unwrap();
        assert_eq!(annotated(&src), dir.join("Shot annotated (2).png"));
    }
}
