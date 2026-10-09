//! Module `capture`: screenshots of an area, a window or a screen through
//! `platform::capture`, saved as PNG files, copied to the clipboard and listed in Recent
//! Captures. Ids are `capture:<key>`: root items `capture:area`, `capture:window`,
//! `capture:screen` and `capture:recent`; in view `capture/recent`, `capture:shot/<row id>`.
//!
//! Table `[capture]`: `dir` (default `~/Pictures/Flick`, created on the first save), `name`
//! (file name template, see `name.rs`), `copy`, `save` (false: a temp file on the clipboard
//! only, not recorded), `sound`, `cursor`, `shadow`, `history` (rows kept; files are never
//! touched by trimming), and `area_hotkey`, `window_hotkey`, `screen_hotkey` (unbound by
//! default). Captures from items and hotkeys run on the worker thread (`run.rs`) and land
//! on `ModuleChanged`; `flick capture screen|display|rect` run on the main thread and answer
//! with the file.
//!
//! Annotation and drawing on the screen (`annotate.rs`) add `colors` (hex, up to 5), `width`,
//! `fade_secs`, `halo_color`, `halo_radius` and `annotate_hotkey`, `draw_hotkey`,
//! `cursor_hotkey`.

mod annotate;
mod args;
mod name;
mod run;
mod store;
mod view;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::Section;
use crate::core::{Action, Binding, Cx, Event, Item, ItemId, ListView, Module, Outcome, unknown_verb};
use crate::platform::capture::{self, Error, Request, Shot, Target};
use crate::platform::{clock, events, files, pasteboard, workspace};
use crate::core::store::{Store, now};
use annotate::{INK_ENV, Ink, InkEnv};
use args::{last, list, options, parse_limit, parse_rect};
use run::{Job, Worker};
use store::{MIGRATIONS, Row, Shots};

/// How long a screen capture started from the launcher waits for the panel to go.
const HIDE_WAIT: Duration = Duration::from_millis(150);

/// The status when Screen Recording is not granted.
const PERMISSION: &str = "Allow Screen Recording for Flick in System Settings";

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
#[expect(clippy::struct_excessive_bools, reason = "the [capture] table's switches, one field each")]
struct Settings {
    dir: String,
    name: String,
    copy: bool,
    save: bool,
    sound: bool,
    cursor: bool,
    shadow: bool,
    history: u32,
    area_hotkey: Option<String>,
    window_hotkey: Option<String>,
    screen_hotkey: Option<String>,
    colors: Vec<String>,
    width: f32,
    fade_secs: f32,
    halo_color: String,
    halo_radius: f64,
    annotate_hotkey: Option<String>,
    draw_hotkey: Option<String>,
    cursor_hotkey: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dir: "~/Pictures/Flick".into(),
            name: name::DEFAULT.into(),
            copy: true,
            save: true,
            sound: true,
            cursor: false,
            shadow: true,
            history: 200,
            area_hotkey: None,
            window_hotkey: None,
            screen_hotkey: None,
            colors: annotate::COLORS.map(String::from).to_vec(),
            width: 4.0,
            fade_secs: 0.0,
            halo_color: annotate::HALO.into(),
            halo_radius: 28.0,
            annotate_hotkey: None,
            draw_hotkey: None,
            cursor_hotkey: None,
        }
    }
}

/// Everything that reaches the system. Tests get fakes that never take a screenshot, write
/// the clipboard, open Finder or use the Trash.
#[derive(Clone, Copy)]
struct Env {
    shoot: fn(&Request) -> Result<Shot, Error>,
    /// Post `ModuleChanged` from the worker thread.
    notify: fn(),
    /// The `Target::Display` number under the mouse (main thread only).
    screen: fn() -> u32,
    permitted: fn() -> bool,
    request_permission: fn() -> bool,
    set_png: fn(&[u8]),
    set_text: fn(&str),
    open: fn(&Path),
    reveal: fn(&Path),
    trash: fn(&Path) -> Result<PathBuf, String>,
    ink: InkEnv,
}

const ENV: Env = Env {
    shoot: if cfg!(test) { |_| Err(Error::Failed("tests never take screenshots".into())) } else { shoot },
    notify: if cfg!(test) { || {} } else { || events::post(Event::ModuleChanged { module: "capture" }) },
    screen: if cfg!(test) { || 1 } else { capture::display_under_mouse },
    permitted: if cfg!(test) { || true } else { capture::permitted },
    request_permission: if cfg!(test) { || false } else { capture::request_permission },
    set_png: if cfg!(test) { |_| {} } else { pasteboard::set_png },
    set_text: if cfg!(test) { |_| {} } else { pasteboard::set_text },
    open: if cfg!(test) { |_| {} } else { workspace::open_file },
    reveal: if cfg!(test) { |_| {} } else { workspace::reveal },
    trash: if cfg!(test) { |_| Err("tests never use the Trash".into()) } else { files::trash },
    ink: INK_ENV,
};

/// The real shutter: make the folder, then run screencapture.
fn shoot(req: &Request) -> Result<Shot, Error> {
    if let Some(dir) = req.path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| Error::Failed(format!("{}: {e}", dir.display())))?;
    }
    capture::run(req)
}

/// A shot as `flick capture` answers it with `--json`.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct Taken {
    path: String,
    width: u32,
    height: u32,
    copied: bool,
}

pub struct Capture {
    settings: Settings,
    /// `settings.dir` with `~` expanded.
    dir: PathBuf,
    worker: Worker,
    env: Env,
    ink: Ink,
}

impl Default for Capture {
    fn default() -> Self {
        let settings = Settings::default();
        let dir = name::expand(&settings.dir, dirs::home_dir().as_deref());
        Capture { settings, dir, worker: Worker::default(), env: ENV, ink: Ink::default() }
    }
}

impl Capture {
    /// Screen Recording is granted; else ask macOS (its prompt shows once) and say why not.
    fn allowed(&self) -> Result<(), String> {
        if (self.env.permitted)() {
            return Ok(());
        }
        (self.env.request_permission)();
        Err(PERMISSION.into())
    }

    /// A job for `target`: into `out`, else a fresh name in `dir` (or the temp dir when
    /// `save` is off, unrecorded).
    fn job(&self, target: Target, out: Option<PathBuf>, copy: bool) -> Job {
        let kind = args::kind(target);
        let record = self.settings.save || out.is_some();
        let path = out.unwrap_or_else(|| {
            let ts = now();
            let file = name::fill(&self.settings.name, kind, ts, clock::utc_offset(ts));
            let dir = if self.settings.save {
                self.dir.clone()
            } else {
                std::env::temp_dir().join("flick-capture")
            };
            name::unique(&dir, &file, Path::exists)
        });
        let s = &self.settings;
        let req = Request { target, path, cursor: s.cursor, shadow: s.shadow, sound: s.sound };
        Job { req, kind, copy, record, annotate: false, wait: Duration::ZERO }
    }

    /// Start an interactive or screen capture on the worker, with the launcher hidden;
    /// `annotate` opens the editor on the shot (which then copies it, not the shutter).
    fn start(&mut self, target: Target, annotate: bool, cx: &mut Cx) -> Outcome {
        if let Err(e) = self.allowed() {
            return Outcome::Stay(Some(e));
        }
        if self.worker.busy() {
            return Outcome::Stay(Some("A capture is already running".into()));
        }
        cx.hide();
        let mut job = self.job(target, None, self.settings.copy);
        job.annotate = annotate;
        if target == Target::Screen {
            // The worker cannot ask AppKit where the mouse is; the launcher must be gone
            // from the picture.
            job.req.target = Target::Display((self.env.screen)());
            job.wait = HIDE_WAIT;
        }
        if let Err(e) = self.worker.start(job, self.env.shoot, self.env.notify) {
            eprintln!("flick: {e}");
        }
        Outcome::Hide
    }

    /// Copy and record a finished shot.
    fn finish(&self, job: &Job, result: Result<Shot, Error>, store: &Store) -> Result<Taken, Error> {
        let shot = result?;
        // An editor opens on a shot to annotate; it copies the result instead.
        let copied = job.copy && !job.annotate && self.copy_image(&shot.path).is_ok();
        if job.record {
            let row = Row {
                id: 0,
                path: shot.path.display().to_string(),
                kind: job.kind.into(),
                width: shot.width,
                height: shot.height,
                taken: now(),
            };
            store.add_shot(&row, self.settings.history);
        }
        let Shot { path, width, height } = shot;
        Ok(Taken { path: path.display().to_string(), width, height, copied })
    }

    /// Put the PNG at `path` on the clipboard.
    fn copy_image(&self, path: &Path) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        (self.env.set_png)(&bytes);
        Ok(())
    }

    /// `screen`, `display <n>` and `rect <x,y,w,h>`: capture on this thread and answer with
    /// the file.
    fn shoot_now(&self, target: Target, opts: &[String], cx: &mut Cx) -> Result<String, String> {
        let opts = options(opts)?;
        if opts.annotate {
            return Err("capture: --annotate works with area and window".into());
        }
        self.allowed().map_err(|e| format!("capture: {e}"))?;
        let job = self.job(target, opts.out, self.settings.copy && !opts.no_copy);
        let result = (self.env.shoot)(&job.req);
        let taken = self.finish(&job, result, cx.store).map_err(|e| format!("capture: {e}"))?;
        if cx.json {
            return serde_json::to_string(&taken).map_err(|e| format!("capture: {e}"));
        }
        Ok(taken.path)
    }

    /// `area` and `window`: start the selection and answer at once.
    fn shoot_later(&self, target: Target, opts: &[String]) -> Result<String, String> {
        let opts = options(opts)?;
        self.allowed().map_err(|e| format!("capture: {e}"))?;
        let mut job = self.job(target, opts.out, self.settings.copy && !opts.no_copy);
        job.annotate = opts.annotate;
        self.worker.start(job, self.env.shoot, self.env.notify).map_err(|e| format!("capture: {e}"))?;
        Ok(if target == Target::Area { "Select an area" } else { "Select a window" }.into())
    }
}


impl Module for Capture {
    fn id(&self) -> &'static str {
        "capture"
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let settings = table.get::<Settings>()?;
        name::validate(&settings.name)?;
        if settings.history == 0 {
            return Err("[capture]: history must be at least 1".into());
        }
        if !settings.save && !settings.copy {
            return Err("[capture]: save and copy are both off, so a capture would go nowhere".into());
        }
        self.ink = Ink::new(&settings)?;
        self.dir = name::expand(&settings.dir, dirs::home_dir().as_deref());
        self.settings = settings;
        Ok(())
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        view::root_items(&self.dir, self.ink_items())
    }

    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        (view == view::RECENT).then(view::recent)
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        view::refresh(view, cx);
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        match id.key() {
            "area" => self.start(Target::Area, false, cx),
            "area-annotate" => self.start(Target::Area, true, cx),
            "window" => self.start(Target::Window, false, cx),
            "screen" => self.start(Target::Screen, false, cx),
            "recent" => Outcome::Push(view::recent()),
            "draw" | "cursor" | "clear" => {
                cx.hide();
                match id.key() {
                    "draw" => self.set_drawing(!(self.env.ink.drawing)()),
                    "cursor" => self.set_cursor(!(self.env.ink.cursor)()),
                    _ => (self.env.ink.clear)(),
                }
                Outcome::Hide
            }
            key => match view::shot_id(key).and_then(|id| cx.store.shot(id)) {
                Some(row) => {
                    cx.hide();
                    (self.env.open)(Path::new(&row.path));
                    Outcome::Hide
                }
                None => Outcome::Stay(None),
            },
        }
    }

    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        if view::shot_id(id.key()).is_some() { view::actions() } else { vec![] }
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        let Some(row) = view::shot_id(id.key()).and_then(|id| cx.store.shot(id)) else {
            return Outcome::Stay(Some("That capture is gone".into()));
        };
        let path = Path::new(&row.path);
        match key {
            "copy-image" => match self.copy_image(path) {
                Ok(()) => Outcome::Stay(Some("Copied image".into())),
                Err(e) => Outcome::Stay(Some(e)),
            },
            "copy-path" => {
                (self.env.set_text)(&row.path);
                Outcome::Stay(Some("Copied path".into()))
            }
            "reveal" => {
                cx.hide();
                (self.env.reveal)(path);
                Outcome::Hide
            }
            "trash" => Outcome::Confirm(view::confirm_trash(&row)),
            "annotate" => match self.edit(path, Some(annotate::annotated(path)), self.settings.copy) {
                Ok(()) => {
                    cx.hide();
                    Outcome::Hide
                }
                Err(e) => Outcome::Stay(Some(e)),
            },
            _ => Outcome::Stay(None),
        }
    }

    fn confirmed(&mut self, token: &str, cx: &mut Cx) -> Outcome {
        let row = token.strip_prefix("trash/").and_then(|id| id.parse().ok()).and_then(|id| cx.store.shot(id));
        let Some(row) = row else { return Outcome::Stay(None) };
        match (self.env.trash)(Path::new(&row.path)) {
            Ok(_) => {
                cx.store.delete_shot(row.id);
                Outcome::Stay(Some("Moved to Trash".into()))
            }
            Err(e) => Outcome::Stay(Some(e)),
        }
    }

    /// Finished captures from the worker: copy and record them, and open the editor on those
    /// to annotate; a closed editor's copy is recorded. Recent Captures is stale.
    fn on_event(&mut self, event: Event, cx: &mut Cx) -> bool {
        if event == Event::DisplaysChanged {
            (self.env.ink.relayout)();
        }
        if event != (Event::ModuleChanged { module: "capture" }) {
            return false;
        }
        for done in self.worker.drain() {
            let shot = self.finish(&done.job, done.result, cx.store);
            let edit = match shot {
                Ok(taken) if done.job.annotate => self.edit(Path::new(&taken.path), None, done.job.copy),
                Ok(_) | Err(Error::Cancelled) => Ok(()),
                Err(Error::Failed(e)) => Err(format!("capture: {e}")),
            };
            if let Err(e) = edit {
                eprintln!("flick: {e}");
            }
        }
        self.edited(cx.store);
        true
    }

    fn hotkeys(&self) -> Vec<Binding> {
        let s = &self.settings;
        [
            ("area", &s.area_hotkey),
            ("window", &s.window_hotkey),
            ("screen", &s.screen_hotkey),
            ("annotate", &s.annotate_hotkey),
            ("draw", &s.draw_hotkey),
            ("cursor", &s.cursor_hotkey),
        ]
        .into_iter()
            .filter_map(|(key, spec)| {
                let spec = spec.as_ref().filter(|s| !s.trim().is_empty())?;
                Some(Binding { spec: spec.clone(), key: Ok(key.into()) })
            })
            .collect()
    }

    /// Starts a capture or toggles drawing or the halo; the launcher stays as it is (hidden).
    fn hotkey(&mut self, key: &str, cx: &mut Cx) -> Option<ListView> {
        let (target, annotate) = match key {
            "area" => (Target::Area, false),
            "annotate" => (Target::Area, true),
            "window" => (Target::Window, false),
            "screen" => (Target::Screen, false),
            "draw" => {
                self.set_drawing(!(self.env.ink.drawing)());
                return None;
            }
            "cursor" => {
                self.set_cursor(!(self.env.ink.cursor)());
                return None;
            }
            _ => return None,
        };
        if let Outcome::Stay(Some(e)) = self.start(target, annotate, cx) {
            eprintln!("flick: capture: {e}");
        }
        None
    }

    fn verbs(&self) -> &'static str {
        "capture area|window [--annotate] | capture screen [--out <path>] [--no-copy] | capture display <n> | capture rect <x,y,w,h> | capture ls [--limit n] | capture last | capture draw on|off|toggle|clear | capture cursor on|off|toggle | capture annotate <path>"
    }

    /// `--json` (`cx.json`) makes `screen`, `display`, `rect` answer `{"path","width",
    /// "height","copied"}`, `ls` an array of rows and `last` one row.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v, rest @ ..] if v == "area" => self.shoot_later(Target::Area, rest),
            [v, rest @ ..] if v == "window" => self.shoot_later(Target::Window, rest),
            [v, rest @ ..] if v == "screen" => self.shoot_now(Target::Screen, rest, cx),
            [v, n, rest @ ..] if v == "display" => {
                let bad = || format!("capture: bad display {n:?}");
                let display = n.parse().ok().filter(|d| *d > 0).ok_or_else(bad)?;
                self.shoot_now(Target::Display(display), rest, cx)
            }
            [v, xywh, rest @ ..] if v == "rect" => self.shoot_now(Target::Rect(parse_rect(xywh)?), rest, cx),
            [v, rest @ ..] if v == "ls" => list(parse_limit(rest)?, cx),
            [v] if v == "last" => last(cx),
            [v, arg] if v == "draw" => self.draw_verb(arg),
            [v, arg] if v == "cursor" => self.cursor_verb(arg),
            [v, path] if v == "annotate" => self.annotate_verb(path),
            _ => Err(unknown_verb("capture", args)),
        }
    }
}

#[cfg(test)]
mod tests;
