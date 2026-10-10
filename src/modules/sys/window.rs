//! The fleet window (flick-a2ed): the fleet on `platform::surface` "fleet", a floating window
//! that stays up while Jaymin works. One bubble per machine (its name, state and how it was
//! read as the header, its age as the time, its metrics and one bullet per service as the
//! text); the header shows `views::summary_line` and a status dot.
//!
//! - `sys window` and cmd+K Open Fleet Window on the root item `Fleet` show it (on the
//!   screen under the pointer, with the keyboard; Flick never activates). Esc or cmd+W hide
//!   it. `sys window --snapshot <png>` draws it, shown or not, into a PNG (mx-6b45b0). Both
//!   are in `NET_DENIED`: a peer must not pop a key window on this Mac or write files.
//! - While it shows the fleet polls as it does for a visible fleet view: every
//!   `VISIBLE_EVERY` s (`Sys::poll`). Hidden, it costs nothing.
//! - The input filters: Return applies its text (machines whose name, state or `via`
//!   holds it with all their services, else only the services whose name, kind or status
//!   does); Return on an empty input shows everything again. cmd+R polls every machine now.
//! - The surface's handlers only queue a `Note` and post `ModuleChanged` (mx-fcbc43); the
//!   module drains them on the main thread, then redraws while it shows.

use std::hash::{DefaultHasher, Hash, Hasher};

use super::fleet::VISIBLE_EVERY;
use super::report::ago;
use super::views::{Seen, Svc, summary_line};
use super::{Poll, Sys};
use crate::platform::surface::rows::{BubbleState, Side};
use crate::platform::surface::{self, Key, Keystroke, Status};

/// What the window needs from the system; `wire::WINDOW` is the real thing.
#[derive(Clone, Copy)]
pub struct Hooks {
    /// Build the window (once; later calls do nothing).
    pub open: fn(),
    /// Show it on the screen under the pointer with the keyboard.
    pub show: fn(),
    pub hide: fn(),
    pub visible: fn() -> bool,
    /// Title, subtitle, status dot.
    pub header: fn(&str, &str, Status),
    pub rows: fn(&[surface::Row]),
    pub notice: fn(Option<&str>),
    /// The notes the window's handlers queued.
    pub take: fn() -> Vec<Note>,
    /// Draw the window into a PNG at a path (`surface::snapshot`).
    pub snapshot: fn(&str) -> Result<(), String>,
}

/// What a window handler queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// Return: the input's text (empty: no filter).
    Filter(String),
    /// cmd+R
    Refresh,
    /// cmd+W
    Close,
}

/// The note for keystroke `k`, reading the input only for Return. `None`: the surface's
/// own fallback (Esc hides).
pub fn note(k: Keystroke, input: impl FnOnce() -> String) -> Option<Note> {
    let plain_cmd = k.cmd && !k.shift && !k.opt;
    match k.key {
        Key::Return => Some(Note::Filter(input())),
        Key::Char('r') if plain_cmd => Some(Note::Refresh),
        Key::Char('w') if plain_cmd => Some(Note::Close),
        _ => None,
    }
}

/// The window's state in the module.
pub struct Win {
    pub hooks: Hooks,
    /// The window was built.
    opened: bool,
    /// The last filter applied, trimmed; empty for none.
    filter: String,
}

impl Win {
    pub fn new(hooks: Hooks) -> Win {
        Win { hooks, opened: false, filter: String::new() }
    }

    /// Whether the window shows.
    pub fn visible(&self) -> bool {
        self.opened && (self.hooks.visible)()
    }

    fn open(&mut self) {
        if !self.opened {
            (self.hooks.open)();
            self.opened = true;
        }
    }
}

/// One machine's bubble, owned until it is handed to the surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bubble {
    pub key: String,
    pub version: u64,
    pub side: Side,
    pub header: String,
    pub time: String,
    pub md: String,
    pub state: BubbleState,
}

impl Bubble {
    fn new(key: String, side: Side, header: String, time: String, md: String, state: BubbleState) -> Bubble {
        let mut h = DefaultHasher::new();
        (&header, &time, &md, format!("{state:?}")).hash(&mut h);
        Bubble { key, version: h.finish(), side, header, time, md, state }
    }

    fn note(key: &str, md: String) -> Bubble {
        Bubble::new(key.into(), Side::System, String::new(), String::new(), md, BubbleState::Done)
    }

    pub fn row(&self) -> surface::Row<'_> {
        surface::Row::Bubble {
            key: &self.key,
            version: self.version,
            side: self.side,
            header: &self.header,
            time: &self.time,
            md: &self.md,
            state: self.state,
        }
    }
}

fn holds(text: &str, needle: &str) -> bool {
    text.to_lowercase().contains(needle)
}

/// One bullet per service: `- **fail** agent · not loaded`.
fn bullet(s: &Svc) -> String {
    let status = if s.status == "ok" { "ok".to_string() } else { format!("**{}**", s.status) };
    format!("- {status} {} · {}", s.name, s.reason)
}

/// Machine `s`'s bubble with the services `filter` (lowercase; empty: none) keeps; `None`
/// when nothing of it matches.
fn machine(s: &Seen, filter: &str) -> Option<Bubble> {
    let m = &s.slot.machine;
    let via = s.row.source.unwrap_or(m.via).as_str();
    let whole = [m.name.as_str(), s.state, via].iter().any(|t| holds(t, filter));
    let services: Vec<&Svc> = s
        .services
        .iter()
        .filter(|svc| whole || [svc.name, svc.kind, svc.status].iter().any(|t| holds(t, filter)))
        .collect();
    if !whole && services.is_empty() {
        return None;
    }
    let header = match s.state {
        "fresh" => format!("{} · via {via}", m.name),
        state => format!("{} · {state} · via {via}", m.name),
    };
    let time = s.age.map_or(String::new(), |a| format!("{} ago", ago(a)));
    let subtitle = s.subtitle();
    let lines = (!subtitle.is_empty()).then_some(subtitle).into_iter().chain(services.into_iter().map(bullet));
    let md = lines.collect::<Vec<_>>().join("\n");
    let state = match s.state {
        "pending" => BubbleState::Pending,
        "down" => BubbleState::Failed,
        _ => BubbleState::Done,
    };
    Some(Bubble::new(format!("machine/{}", m.name), Side::Theirs, header, time, md, state))
}

/// The window's bubbles: each machine `filter` (as typed) keeps, or a note why none shows.
pub fn bubbles(seen: &[Seen], filter: &str) -> Vec<Bubble> {
    if seen.is_empty() {
        return vec![Bubble::note("empty", "No machines (add `[[sys.machine]]` tables to config.toml)".into())];
    }
    let needle = filter.trim().to_lowercase();
    let out: Vec<Bubble> = seen.iter().filter_map(|s| machine(s, &needle)).collect();
    if out.is_empty() {
        return vec![Bubble::note("none", format!("Nothing matches “{}”", filter.trim()))];
    }
    out
}

/// The header's subtitle and dot: red when a machine is down or a service fails, orange
/// while a round runs or a machine was never read, else idle.
pub fn header(seen: &[Seen], busy: bool) -> (String, Status) {
    let down = seen.iter().any(|s| s.state == "down" || s.services.iter().any(|v| v.status == "fail"));
    let pending = busy || seen.iter().any(|s| s.state == "pending");
    let status = if down {
        Status::Error
    } else if pending {
        Status::Busy
    } else {
        Status::Idle
    };
    (format!("{}  ·  ⌘R refresh  ·  Esc hides", summary_line(seen)), status)
}

/// The notice line for `filter`.
pub fn notice(filter: &str) -> Option<String> {
    (!filter.is_empty()).then(|| format!("Showing “{filter}”  ·  Return on an empty field shows all"))
}

const USAGE: &str = "sys: usage: sys window [--snapshot <png>]";

impl Sys {
    /// Show the window and poll as a visible fleet view does.
    pub(super) fn window_show(&mut self) {
        self.win.open();
        self.window_draw();
        (self.win.hooks.show)();
        if self.started {
            self.poll(Poll::Open);
        }
    }

    /// Handle what the window queued.
    pub(super) fn window_drain(&mut self) {
        for note in (self.win.hooks.take)() {
            match note {
                Note::Filter(text) => text.trim().clone_into(&mut self.win.filter),
                Note::Refresh => self.poll(Poll::Now),
                Note::Close => (self.win.hooks.hide)(),
            }
        }
    }

    /// Redraw the window if it shows.
    pub(super) fn window_refresh(&self) {
        if self.win.visible() {
            self.window_draw();
        }
    }

    fn window_draw(&self) {
        let busy = self.shared.lock().fleet.busy();
        let (bubbles, (subtitle, status)) =
            self.with_seen(|seen, _| (bubbles(seen, &self.win.filter), header(seen, busy)));
        let rows: Vec<surface::Row> = bubbles.iter().map(Bubble::row).collect();
        (self.win.hooks.rows)(&rows);
        (self.win.hooks.header)("Fleet", &subtitle, status);
        (self.win.hooks.notice)(notice(&self.win.filter).as_deref());
    }

    /// `sys window`: show it. `sys window --snapshot <png>`: draw it into a PNG.
    pub(super) fn window_command(&mut self, args: &[String]) -> Result<String, String> {
        match args {
            [] => {
                self.window_show();
                Ok(format!("Fleet window shown (polls every {VISIBLE_EVERY} s while it shows)"))
            }
            [flag, path] if flag == "--snapshot" => {
                self.win.open();
                self.window_draw();
                (self.win.hooks.snapshot)(path)?;
                Ok(format!("Wrote {path}"))
            }
            _ => Err(USAGE.into()),
        }
    }
}
