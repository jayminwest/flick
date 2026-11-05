//! Workspaces: eight numbered workspaces, each pinned to a screen by an ordered list of
//! screen matches; the one visible workspace per screen; the focused one and the one before
//! it (back-and-forth); and which workspace each managed window is on. Pure Rust: switches
//! return a `Plan` of windows for the caller to show and hide.

use super::WinId;
use crate::platform::Rect;

/// How many workspaces there are: a fixed list, numbered 1 to `COUNT`.
pub const COUNT: usize = 8;

/// One of the `COUNT` workspaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ws(u8);

impl Ws {
    /// Workspace `number`, counted from 1.
    pub fn new(number: usize) -> Option<Ws> {
        (1..=COUNT).contains(&number).then(|| Ws(number as u8 - 1))
    }

    /// Every workspace, 1 first.
    pub fn all() -> impl Iterator<Item = Ws> {
        (0..COUNT as u8).map(Ws)
    }

    /// Its number, counted from 1.
    pub fn number(self) -> usize {
        usize::from(self.0) + 1
    }

    /// The next workspace; after the last comes 1.
    pub fn next(self) -> Ws {
        Ws((self.0 + 1) % COUNT as u8)
    }

    /// The previous workspace; before 1 comes the last.
    pub fn prev(self) -> Ws {
        Ws((self.0 + COUNT as u8 - 1) % COUNT as u8)
    }

    fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// A screen as tile sees it. The main screen (the one with the menu bar) comes first.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    /// The display id (CGDirectDisplayID): two screens can share a name.
    pub id: u32,
    /// The display's name, e.g. "DELL U2723QE".
    pub name: String,
    /// The Mac's own display.
    pub built_in: bool,
    /// The usable area (minus menu bar and Dock), in Accessibility coordinates.
    pub area: Rect,
}

/// One entry of a workspace's screen pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match {
    /// `built-in`: the Mac's own display.
    BuiltIn,
    /// `main`: the screen with the menu bar.
    Main,
    /// `secondary`: the screen that is not main, when there are exactly two.
    Secondary,
    /// Any other text: a screen whose name contains it, ignoring case.
    Name(String),
}

impl Match {
    pub fn parse(text: &str) -> Match {
        match text {
            "built-in" => Match::BuiltIn,
            "main" => Match::Main,
            "secondary" => Match::Secondary,
            name => Match::Name(name.to_lowercase()),
        }
    }

    /// The index of the first of `screens` this matches.
    fn find(&self, screens: &[Screen]) -> Option<usize> {
        match self {
            Match::BuiltIn => screens.iter().position(|s| s.built_in),
            Match::Main => (!screens.is_empty()).then_some(0),
            Match::Secondary => (screens.len() == 2).then_some(1),
            Match::Name(name) => screens.iter().position(|s| s.name.to_lowercase().contains(name)),
        }
    }
}

/// Windows to move on screen and off it, in this order: showing the target first and
/// hiding the old workspace second flickers less.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub show: Vec<WinId>,
    pub hide: Vec<WinId>,
}

/// The workspaces of every screen, and the windows on them.
#[derive(Clone, Debug)]
pub struct Workspaces {
    pins: [Vec<Match>; COUNT],
    screens: Vec<Screen>,
    /// `shown[i]` is visible on `screens[i]`.
    shown: Vec<Ws>,
    focused: Ws,
    last: Option<Ws>,
    windows: Vec<(WinId, Ws)>,
}

impl Workspaces {
    /// No screens yet (`set_screens` adds them) and no windows. `pins[i]` is workspace
    /// i + 1's screen pin; an empty pin shows the workspace on whichever screen has focus.
    pub fn new(pins: [Vec<Match>; COUNT]) -> Workspaces {
        Workspaces {
            pins,
            screens: vec![],
            shown: vec![],
            focused: Ws(0),
            last: None,
            windows: vec![],
        }
    }

    /// The screen `ws` is pinned to: its first pin match, else the built-in screen, else
    /// the main one. `None` when it has no pin (or there are no screens).
    pub fn home(&self, ws: Ws) -> Option<usize> {
        let pin = &self.pins[ws.index()];
        if pin.is_empty() || self.screens.is_empty() {
            return None;
        }
        let built_in = || Match::BuiltIn.find(&self.screens);
        Some(pin.iter().find_map(|m| m.find(&self.screens)).or_else(built_in).unwrap_or(0))
    }

    /// Take a new screen list (at most `COUNT` screens; an empty list is ignored, as macOS
    /// reports none only mid-change). Each screen keeps its workspace when that still
    /// belongs there; a screen without one gets a workspace pinned to it, else an unpinned
    /// one, else any, preferring workspaces that were visible.
    pub fn set_screens(&mut self, mut screens: Vec<Screen>) -> Plan {
        if screens.is_empty() {
            return Plan::default();
        }
        screens.truncate(COUNT);
        let before = std::mem::take(&mut self.shown);
        let old: Vec<(u32, Ws)> =
            self.screens.iter().map(|s| s.id).zip(before.iter().copied()).collect();
        self.screens = screens;
        let mut shown: Vec<Option<Ws>> = self
            .screens
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let (_, ws) = old.iter().find(|(id, _)| *id == s.id)?;
                self.home(*ws).is_none_or(|h| h == i).then_some(*ws)
            })
            .collect();
        let candidates: Vec<Ws> = before.iter().copied().chain(Ws::all()).collect();
        for i in 0..shown.len() {
            if shown[i].is_some() {
                continue;
            }
            let free = |ws: &&Ws| !shown.contains(&Some(**ws));
            let fits = |want: Option<usize>| {
                candidates.iter().filter(free).find(|ws| self.home(**ws) == want).copied()
            };
            let any = || candidates.iter().find(free).copied();
            shown[i] = fits(Some(i)).or_else(|| fits(None)).or_else(any);
        }
        self.shown = shown.into_iter().flatten().collect();
        if !self.shown.contains(&self.focused) {
            self.focused = self.shown[0];
        }
        let changed = |from: &[Ws], to: &[Ws]| {
            self.windows
                .iter()
                .filter(|(_, ws)| to.contains(ws) && !from.contains(ws))
                .map(|(w, _)| *w)
                .collect()
        };
        Plan { show: changed(&before, &self.shown), hide: changed(&self.shown, &before) }
    }

    pub fn screens(&self) -> &[Screen] {
        &self.screens
    }

    /// The focused workspace; it is always visible once there are screens.
    pub fn focused(&self) -> Ws {
        self.focused
    }

    /// The workspace focused before this one (back-and-forth goes there).
    pub fn last(&self) -> Option<Ws> {
        self.last
    }

    /// The screen showing the focused workspace.
    pub fn focused_screen(&self) -> Option<usize> {
        self.screen_of(self.focused)
    }

    /// The screen `ws` is visible on.
    pub fn screen_of(&self, ws: Ws) -> Option<usize> {
        self.shown.iter().position(|s| *s == ws)
    }

    /// The workspace visible on screen `i`.
    pub fn shown_on(&self, i: usize) -> Option<Ws> {
        self.shown.get(i).copied()
    }

    /// Focus workspace `target`. One visible on a screen just takes focus there (so a
    /// workspace pinned to another screen focuses that screen). Otherwise it replaces the
    /// workspace on its home screen, or on the focused screen when it has no pin.
    pub fn switch(&mut self, target: Ws) -> Plan {
        let Some(focused) = self.focused_screen().filter(|_| target != self.focused) else {
            return Plan::default();
        };
        let plan = if self.screen_of(target).is_some() {
            Plan::default()
        } else {
            let i = self.home(target).unwrap_or(focused);
            let old = std::mem::replace(&mut self.shown[i], target);
            Plan { show: self.windows_on(target), hide: self.windows_on(old) }
        };
        self.last = Some(self.focused);
        self.focused = target;
        plan
    }

    /// The workspace after the focused one, wrapping around all `COUNT`.
    pub fn next(&mut self) -> Plan {
        self.switch(self.focused.next())
    }

    /// The workspace before the focused one, wrapping around all `COUNT`.
    pub fn prev(&mut self) -> Plan {
        self.switch(self.focused.prev())
    }

    /// Back to the workspace focused last.
    pub fn back_and_forth(&mut self) -> Plan {
        self.last.map(|ws| self.switch(ws)).unwrap_or_default()
    }

    /// Focus the workspace visible on screen `i` (focus crossed to that screen).
    pub fn focus_screen(&mut self, i: usize) {
        if let Some(ws) = self.shown_on(i).filter(|ws| *ws != self.focused) {
            self.last = Some(self.focused);
            self.focused = ws;
        }
    }

    /// Put window `w` on `ws`, a new window or a move. It is hidden when it leaves the
    /// screen (a new window counts as on screen) and shown when it comes back.
    pub fn put(&mut self, w: WinId, ws: Ws) -> Plan {
        let was = self.workspace_of(w).is_none_or(|old| self.screen_of(old).is_some());
        self.remove(w);
        self.windows.push((w, ws));
        match (was, self.screen_of(ws).is_some()) {
            (true, false) => Plan { show: vec![], hide: vec![w] },
            (false, true) => Plan { show: vec![w], hide: vec![] },
            _ => Plan::default(),
        }
    }

    /// Forget window `w` (it closed).
    pub fn remove(&mut self, w: WinId) {
        self.windows.retain(|(id, _)| *id != w);
    }

    /// The workspace window `w` is on.
    pub fn workspace_of(&self, w: WinId) -> Option<Ws> {
        self.windows.iter().find(|(id, _)| *id == w).map(|(_, ws)| *ws)
    }

    /// The windows on `ws`, in the order they were put there.
    pub fn windows_on(&self, ws: Ws) -> Vec<WinId> {
        self.windows.iter().filter(|(_, on)| *on == ws).map(|(w, _)| *w).collect()
    }
}

#[cfg(test)]
mod tests;
