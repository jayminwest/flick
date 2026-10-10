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
//!   does); Return on an empty input shows everything again. cmd+R polls every machine now
//!   (flick-4e70): the notice says `Refreshing…` while that round runs, then `Refreshed`
//!   until the first redraw `REFRESHED_FOR` s later (at the latest the next 15 s tick),
//!   ahead of the filter's notice.
//! - cmd+K (flick-1e00) shows or hides an action card under each machine's bubble
//!   (`menu.rs`): Screen Sharing, Open Dash, Tail <service> and Restart <service>…, as the
//!   launcher's cmd+K offers them. A tail shows as a bubble under that machine's card;
//!   Restart asks first on the card (the exact command, Cancel and Run). The notice line
//!   says what a press started or why it could not (`State::acted`, `Win::said`).
//! - `[sys] hotkey` shows the window, or hides it when it has the keyboard (flick-1e00).
//! - The rows sit at the top (`surface::Align::Top`): a dashboard, not a chat.
//! - The surface's handlers only queue a `Note` and post `ModuleChanged` (mx-fcbc43); the
//!   module drains them on the main thread, then redraws while it shows.

use std::hash::{DefaultHasher, Hash, Hasher};

use super::fleet::VISIBLE_EVERY;
use super::jobs::{self, Tail};
use super::report::ago;
use super::views::{self, Seen, Svc, summary_line};
use super::{Poll, Sys, act, fleet, local, menu};
use crate::core::card::Card;
use crate::platform::hud::CardUi;
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
    /// It has the keyboard.
    pub key: fn() -> bool,
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
    /// cmd+K: show or hide the action cards.
    Actions,
    /// A button on an action card: its card id and action id.
    Press { card: String, action: String },
}

/// The note for keystroke `k`, reading the input only for Return. `None`: the surface's
/// own fallback (Esc hides).
pub fn note(k: Keystroke, input: impl FnOnce() -> String) -> Option<Note> {
    let plain_cmd = k.cmd && !k.shift && !k.opt;
    match k.key {
        Key::Return => Some(Note::Filter(input())),
        Key::Char('r') if plain_cmd => Some(Note::Refresh),
        Key::Char('w') if plain_cmd => Some(Note::Close),
        Key::Char('k') if plain_cmd => Some(Note::Actions),
        _ => None,
    }
}

/// How long `Refreshed` stays after a cmd+R round, in seconds.
pub const REFRESHED_FOR: u64 = 3;

/// Where the last cmd+R stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refresh {
    Idle,
    /// Its round runs.
    Running,
    /// Its round ended at this time.
    Done(u64),
}

/// The window's state in the module.
pub struct Win {
    pub hooks: Hooks,
    /// The window was built.
    opened: bool,
    /// The last filter applied, trimmed; empty for none.
    filter: String,
    refresh: Refresh,
    /// cmd+K: the action cards show.
    pub(super) actions: bool,
    /// The restart waiting on its card's Run: card id and action id.
    pub(super) confirm: Option<(String, String)>,
    /// A tail asked for from a card: the last tail shows under its machine's card.
    pub(super) tail: bool,
    /// Why the last press did nothing, for the notice line.
    pub(super) said: Option<String>,
}

impl Win {
    pub fn new(hooks: Hooks) -> Win {
        Win {
            hooks,
            opened: false,
            filter: String::new(),
            refresh: Refresh::Idle,
            actions: false,
            confirm: None,
            tail: false,
            said: None,
        }
    }

    /// Hide the action cards and what they started showing.
    fn reset_actions(&mut self) {
        self.actions = false;
        self.confirm = None;
        self.tail = false;
        self.said = None;
    }

    /// Move the cmd+R state on: a round that ran and no longer `busy` is `Done` at `now`;
    /// `Done` ends `REFRESHED_FOR` s later. The notice text, if any.
    fn refresh_notice(&mut self, busy: bool, now: u64) -> Option<&'static str> {
        self.refresh = match self.refresh {
            Refresh::Running if !busy => Refresh::Done(now),
            Refresh::Done(at) if now.saturating_sub(at) >= REFRESHED_FOR => Refresh::Idle,
            other => other,
        };
        match self.refresh {
            Refresh::Idle => None,
            Refresh::Running => Some("Refreshing…"),
            Refresh::Done(_) => Some("Refreshed"),
        }
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

/// A row the window owns until it hands it to the surface.
#[derive(Clone, Debug, PartialEq)]
pub enum Shown {
    Bubble(Bubble),
    /// A machine's action card; `confirm` names its restart waiting on Run.
    Card { card: Card, version: u64, confirm: Option<String> },
}

impl Shown {
    fn card(card: Card, confirm: Option<String>) -> Shown {
        let mut h = DefaultHasher::new();
        (format!("{card:?}"), &confirm).hash(&mut h);
        Shown::Card { card, version: h.finish(), confirm }
    }

    pub fn row(&self) -> surface::Row<'_> {
        match self {
            Shown::Bubble(b) => b.row(),
            Shown::Card { card, version, confirm } => {
                let ui = CardUi { confirm: confirm.as_deref(), ..CardUi::default() };
                surface::Row::Card { key: &card.id, version: *version, card, ui }
            }
        }
    }
}

/// The last tail as a bubble: its lines as code, or why there are none.
pub fn tail_bubble(t: &Tail, now: u64) -> Bubble {
    let header = format!("Tail · {} on {}", t.service, t.machine);
    let time = t.at.map_or(String::new(), |at| format!("{} ago", ago(now.saturating_sub(at))));
    let (md, state) = match &t.text {
        None => (format!("Running `{}`…", t.shown), BubbleState::Pending),
        Some(Ok(lines)) if lines.trim().is_empty() => ("The log is empty.".to_string(), BubbleState::Done),
        Some(Ok(lines)) => (format!("```\n{}\n```", lines.trim_end()), BubbleState::Done),
        Some(Err(e)) => (e.clone(), BubbleState::Failed),
    };
    Bubble::new("tail".into(), Side::Theirs, header, time, md, state)
}

/// `bubbles` with, under each machine's, its card from `cards` (by machine name) and then
/// the tail bubble when `tail` names that machine.
pub fn interleave(bubbles: Vec<Bubble>, mut cards: Vec<(String, Shown)>, mut tail: Option<(&str, Bubble)>) -> Vec<Shown> {
    let mut out = vec![];
    for b in bubbles {
        let name = b.key.strip_prefix("machine/").map(str::to_string);
        out.push(Shown::Bubble(b));
        let Some(name) = name else { continue };
        if let Some(i) = cards.iter().position(|(m, _)| *m == name) {
            out.push(cards.remove(i).1);
        }
        if tail.as_ref().is_some_and(|(m, _)| *m == name) {
            out.extend(tail.take().map(|(_, b)| Shown::Bubble(b)));
        }
    }
    out
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
    (format!("{}  ·  ⌘K actions  ·  ⌘R refresh  ·  Esc hides", summary_line(seen)), status)
}

/// The notice line: the cmd+R state `refresh`, what an action did (`status`), then what
/// `filter` shows.
pub fn notice(refresh: Option<&str>, status: Option<&str>, filter: &str) -> Option<String> {
    let filter = (!filter.is_empty()).then(|| format!("Showing “{filter}”  ·  Return on an empty field shows all"));
    let parts: Vec<&str> = [refresh, status, filter.as_deref()].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join("  ·  "))
}

const USAGE: &str = "sys: usage: sys window [--snapshot <png>]";

/// The notice while cmd+K shows no card.
const NO_ACTIONS: &str = "No actions: set vnc or dash on a machine, or log or restart on a service";

impl Sys {
    /// Show the window and poll as a visible fleet view does.
    pub(super) fn window_show(&mut self) {
        if !self.win.visible() {
            self.win.reset_actions();
        }
        self.win.open();
        self.window_draw();
        (self.win.hooks.show)();
        if self.started {
            self.poll(Poll::Open);
        }
    }

    /// The hotkey: show the window, or hide it when it has the keyboard.
    pub(super) fn window_toggle(&mut self) {
        if self.win.visible() && (self.win.hooks.key)() {
            self.window_hide();
        } else {
            self.window_show();
        }
    }

    fn window_hide(&mut self) {
        (self.win.hooks.hide)();
        self.win.refresh = Refresh::Idle;
    }

    /// Handle what the window queued.
    pub(super) fn window_drain(&mut self) {
        for note in (self.win.hooks.take)() {
            match note {
                Note::Filter(text) => text.trim().clone_into(&mut self.win.filter),
                Note::Refresh => {
                    self.poll(Poll::Now);
                    self.win.refresh = Refresh::Running;
                }
                Note::Close => self.window_hide(),
                Note::Actions => {
                    let on = !self.win.actions;
                    self.win.reset_actions();
                    self.win.actions = on;
                }
                Note::Press { card, action } => self.window_press(&card, &action),
            }
        }
    }

    /// Each machine's action card by name, with the confirm its restart waits on.
    fn window_cards(&self, seen: &[Seen], st: &super::io::State) -> Vec<(String, Shown)> {
        let uid = (self.hooks.uid)();
        let confirm = self.win.confirm.as_ref();
        let card = |s: &Seen| {
            let m = &s.slot.machine;
            let resolve = |svc: &Svc| act::resolve(&st.fleet, &st.services, Some(&m.name), svc.name).ok();
            let targets: Vec<_> = s
                .services
                .iter()
                .filter_map(resolve)
                .map(|t| {
                    let shown = t.restart(uid).map(|run| run.shown);
                    (t, shown)
                })
                .collect();
            let card = menu::card(m, &targets)?;
            let waits = confirm.filter(|(id, _)| *id == card.id).map(|(_, action)| action.clone());
            Some((m.name.clone(), Shown::card(card, waits)))
        };
        seen.iter().filter_map(card).collect()
    }

    /// Redraw the window if it shows.
    pub(super) fn window_refresh(&mut self) {
        if self.win.visible() {
            self.window_draw();
        }
    }

    fn window_draw(&mut self) {
        let (busy, reading) = {
            let st = self.shared.lock();
            let local = st.fleet.has_local() && (st.probing || st.checking);
            (st.fleet.busy(), st.fleet.busy() || local)
        };
        let now = (self.hooks.now)();
        let refresh = self.win.refresh_notice(reading, now);
        let (shown, (subtitle, status), said) = {
            let st = self.shared.lock();
            let local = local(&st, now);
            let seen = views::seen(&st.fleet, &local, now, fleet::stale_after(self.refresh_secs));
            let bubbles = bubbles(&seen, &self.win.filter);
            let (shown, said) = if self.win.actions {
                let cards = self.window_cards(&seen, &st);
                let tail = st.tail.as_ref().filter(|_| self.win.tail).map(|t| (t.machine.as_str(), tail_bubble(t, now)));
                let none = (cards.is_empty() && !seen.is_empty()).then_some(NO_ACTIONS);
                (interleave(bubbles, cards, tail), self.win.said.as_deref().or(none))
            } else {
                (bubbles.into_iter().map(Shown::Bubble).collect(), None)
            };
            let said = said.or_else(|| jobs::acted(st.acted.as_ref(), now)).map(str::to_string);
            (shown, header(&seen, busy), said)
        };
        let rows: Vec<surface::Row> = shown.iter().map(Shown::row).collect();
        (self.win.hooks.rows)(&rows);
        (self.win.hooks.header)("Fleet", &subtitle, status);
        (self.win.hooks.notice)(notice(refresh, said.as_deref(), &self.win.filter).as_deref());
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
