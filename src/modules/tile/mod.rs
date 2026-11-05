//! tile: Flick's tiling window manager, replacing `AeroSpace` (plan pl-41fb). Opt-in
//! (`modules::OPT_IN`): registered only when `[tile]` sets a key.
//!
//! This is the spike (flick-d330), enough to judge speed and safety. Workspaces 1..N are flat
//! lists of window ids (`spike`); new windows join the focused one. `tile ws N` puts N's
//! windows back, then parks the old one's at a screen corner (`platform::winman::hide`), and
//! logs how long until every app applied both. Crash safety: a window's row goes into
//! `tile_hidden` (`store`) before it is parked and leaves once it is back. Quit (menu, `quit`,
//! SIGTERM) and a reload that turns tile off put every parked window back. After a `kill -9`,
//! `Started` keeps the rows whose window still exists and puts each back as winman reports
//! it; any other window that turns up parked at a corner is centred on its screen. A parked
//! window that winman's snapshot loses but the window server still has keeps its row the same
//! and its workspace the same way: it returns still parked, and goes back to its old frame
//! once its workspace is focused. Real platform calls sit behind
//! `wire::Hooks`.
//!
//! Pure cores for the next steps, not wired in yet (flick-ac1d): the split tree of a workspace
//! (`tree`) and the frames it gives its windows (`layout`), workspaces and screen pins
//! (`workspace`), app pins and float rules (`rules`) and directional neighbours (`nav`).

#[cfg_attr(not(test), expect(dead_code, reason = "the tile module wires it in flick-ac1d"))]
pub mod layout;
#[cfg_attr(not(test), expect(dead_code, reason = "the tile module wires it in flick-ac1d"))]
pub mod nav;
#[cfg_attr(not(test), expect(dead_code, reason = "the tile module wires it in flick-ac1d"))]
pub mod rules;
mod spike;
mod store;
#[cfg(test)]
mod tests;
#[cfg_attr(not(test), expect(dead_code, reason = "the tile module wires it in flick-ac1d"))]
pub mod tree;
mod wire;
#[cfg_attr(not(test), expect(dead_code, reason = "the tile module wires it in flick-ac1d"))]
pub mod workspace;

/// A window, by its window-server id: winman's `WinId`, which every part of tile uses.
pub use crate::platform::winman::WinId;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use crate::config::Section;
use crate::core::store::Store;
use crate::core::{Cx, Event, Module, unknown_verb};
use crate::platform::Rect;
use crate::platform::winman::hide::Batch;
use crate::platform::winman::WinInfo;
use spike::Spaces;
use store::{MIGRATIONS, Parked, TileStore};
use wire::Hooks;

const ID: &str = "tile";
const NOT_RUNNING: &str = "tile: not running: Flick needs Accessibility permission";
/// How long putting windows back at quit waits for their apps.
const QUIT_WAIT: Duration = Duration::from_secs(2);

/// The `[tile]` table.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    /// Workspaces 1..=`workspaces`.
    workspaces: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { workspaces: 8 }
    }
}

pub struct Tile {
    hooks: Hooks,
    spaces: Spaces,
    /// `Started` arrived (a reload builds throwaway instances that never start).
    started: bool,
    /// winman runs: Accessibility permission was there at `Started`.
    running: bool,
    /// flick.db's path, for `Drop`; None for an in-memory store.
    db: Option<String>,
    /// Windows this run parked, by id, as in `tile_hidden`; some may be missing from the
    /// snapshot for a while.
    parked: BTreeMap<WinId, Parked>,
    /// Rows a killed Flick left whose window still exists but winman has not reported yet.
    leftover: BTreeMap<WinId, Parked>,
    /// Every window seen in a snapshot and not gone since.
    seen: BTreeSet<WinId>,
    /// The last switch, while its frame sets are out.
    switching: Option<Switching>,
}

struct Switching {
    to: usize,
    at: Instant,
    windows: usize,
    batches: [Batch; 2],
}

impl Default for Tile {
    fn default() -> Self {
        Tile::new(wire::HOOKS)
    }
}

/// Teardown (a reload turned tile off): put every parked window back, then stop watching.
impl Drop for Tile {
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        if let Some(db) = &self.db {
            (self.hooks.put_back)(db);
        }
        (self.hooks.stop)();
    }
}

impl Tile {
    fn new(hooks: Hooks) -> Tile {
        Tile {
            hooks,
            spaces: Spaces::new(Settings::default().workspaces.into()),
            started: false,
            running: false,
            db: None,
            parked: BTreeMap::new(),
            leftover: BTreeMap::new(),
            seen: BTreeSet::new(),
            switching: None,
        }
    }

    fn log(&self, line: &str) {
        (self.hooks.log)(line);
    }

    /// Hook quit, keep the rows of a killed Flick whose windows still exist, and start
    /// watching windows.
    fn start(&mut self, cx: &mut Cx) {
        if self.started {
            return;
        }
        self.started = true;
        self.db = cx.store.conn().path().filter(|p| !p.is_empty()).map(str::to_owned);
        (self.hooks.on_quit)(cx.store);
        let (live, gone): (Vec<Parked>, Vec<Parked>) =
            cx.store.parked_all().into_iter().partition(|r| (self.hooks.exists)(r.id));
        cx.store.parked_delete(&gone.iter().map(|r| r.id).collect::<Vec<_>>());
        if !live.is_empty() {
            self.log(&format!("putting back {} windows a previous run left parked", live.len()));
        }
        self.leftover = live.into_iter().map(|r| (r.id, r)).collect();
        self.running = (self.hooks.start)();
        if !self.running {
            self.log("needs Accessibility permission; windows stay where they are");
            return;
        }
        self.sync(cx);
    }

    /// Catch up with winman's windows: forget closed ones, put back leftover and stray ones,
    /// and add new ones to the focused workspace.
    fn sync(&mut self, cx: &mut Cx) {
        let windows = (self.hooks.snapshot)();
        let live: HashSet<WinId> = windows.iter().map(|w| w.id).collect();
        let mut forget = vec![];
        // The snapshot can miss a window for a while (AXWindows leaves out other Spaces'): a
        // parked one keeps its row and its workspace, a leftover one its row, until it is back
        // or really gone.
        let exists = self.hooks.exists;
        let mut closed: Vec<WinId> = self.leftover.keys().copied().collect();
        closed.extend(self.parked.keys());
        closed.retain(|id| !live.contains(id) && !exists(*id));
        for &id in &closed {
            self.leftover.remove(&id);
            self.parked.remove(&id);
        }
        let mut gone = closed.clone();
        for id in self.seen.iter().copied().filter(|id| !live.contains(id)).collect::<Vec<_>>() {
            self.seen.remove(&id);
            if !self.parked.contains_key(&id) {
                gone.push(id);
            }
        }
        for id in gone {
            self.spaces.remove(id);
        }
        forget.extend(closed);
        let mut moves = vec![];
        for w in &windows {
            if !self.seen.insert(w.id) {
                continue;
            }
            // A parked window back from a gap stays parked on its workspace.
            if let Some(row) = self.leftover.remove(&w.id) {
                moves.push((w.id, row.frame));
                forget.push(w.id);
            } else if !self.parked.contains_key(&w.id)
                && let Some(frame) = (self.hooks.stray)(w.frame)
            {
                moves.push((w.id, frame));
            }
            if managed(w) {
                self.spaces.add(w.id);
            }
        }
        // The focused workspace never has parked windows winman reports (a smaller
        // `workspaces`, a switch or `restore` while one was missing moves them there).
        for &id in self.spaces.windows(self.spaces.focused()).iter().filter(|id| live.contains(id)) {
            if let Some(row) = self.parked.remove(&id) {
                moves.push((id, row.frame));
                forget.push(id);
            }
        }
        if !moves.is_empty() {
            (self.hooks.place)(&moves);
        }
        cx.store.parked_delete(&forget);
        self.report();
    }

    /// `tile ws <n>`: put workspace n's windows back, then park the focused one's.
    fn switch(&mut self, arg: &str, cx: &mut Cx) -> Result<String, String> {
        let count = self.spaces.count();
        let n = arg
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=count).contains(n))
            .ok_or_else(|| format!("tile: ws: expected a workspace 1-{count}"))?;
        if !self.running {
            return Err(NOT_RUNNING.into());
        }
        let at = Instant::now();
        let windows: BTreeMap<WinId, WinInfo> =
            (self.hooks.snapshot)().into_iter().map(|w| (w.id, w)).collect();
        let from = self.spaces.focused();
        let Some(switch) = self.spaces.switch(n - 1) else {
            return Ok(format!("workspace {n}"));
        };
        let rows: Vec<Parked> = switch
            .hide
            .iter()
            .filter_map(|id| windows.get(id))
            .filter(|w| !w.minimized && !w.fullscreen && !self.parked.contains_key(&w.id))
            .map(|w| Parked {
                id: w.id,
                pid: w.app.pid,
                bundle_id: w.app.bundle_id.clone(),
                frame: w.frame,
            })
            .collect();
        if let Err(e) = cx.store.parked_put(&rows) {
            self.spaces.switch(from);
            return Err(e);
        }
        // A parked window winman does not report stays parked: its row puts it back when it
        // returns (flick-cb18).
        let show: Vec<(WinId, Rect)> = switch
            .show
            .iter()
            .filter(|id| windows.contains_key(id))
            .filter_map(|id| self.parked.remove(id))
            .map(|r| (r.id, r.frame))
            .collect();
        let shown = (self.hooks.place)(&show);
        cx.store.parked_delete(&show.iter().map(|&(id, _)| id).collect::<Vec<_>>());
        let hide: Vec<(WinId, Rect)> = rows.iter().map(|r| (r.id, r.frame)).collect();
        let hidden = (self.hooks.hide)(&hide);
        self.parked.extend(rows.into_iter().map(|r| (r.id, r)));
        let windows = show.len() + hide.len();
        self.switching = Some(Switching { to: n, at, windows, batches: [shown, hidden] });
        self.report();
        Ok(format!("workspace {n}: {} shown, {} hidden", show.len(), hide.len()))
    }

    /// Log the last switch's time once every app applied its frame sets.
    fn report(&mut self) {
        let Some(s) = &self.switching else { return };
        let [Some(a), Some(b)] = s.batches.each_ref().map(Batch::finished) else { return };
        let ms = a.max(b).saturating_duration_since(s.at).as_millis();
        self.log(&format!("ws {}: {} windows in {ms} ms", s.to, s.windows));
        self.switching = None;
    }

    /// `tile restore`: put every parked window back now, all on the focused workspace. One
    /// winman does not report keeps its row and goes back when it returns (flick-cb18).
    fn restore(&mut self, cx: &mut Cx) -> Result<String, String> {
        if !self.running {
            return Err(NOT_RUNNING.into());
        }
        let live: HashSet<WinId> = (self.hooks.snapshot)().iter().map(|w| w.id).collect();
        let frames: Vec<(WinId, Rect)> = self
            .parked
            .values()
            .filter(|r| live.contains(&r.id))
            .map(|r| (r.id, r.frame))
            .collect();
        (self.hooks.place)(&frames);
        let back: Vec<WinId> = frames.iter().map(|&(id, _)| id).collect();
        for id in &back {
            self.parked.remove(id);
        }
        cx.store.parked_delete(&back);
        self.spaces.gather();
        Ok(format!("put back {} windows", frames.len()))
    }

    /// `tile ls [--json]`: every window winman knows, by workspace.
    fn ls(&self, json: bool) -> Result<String, String> {
        if !self.running {
            return Err(NOT_RUNNING.into());
        }
        let mut windows = (self.hooks.snapshot)();
        let ws = |w: &WinInfo| self.spaces.of(w.id).map(|i| i + 1);
        windows.sort_by_key(|w| (ws(w).is_none(), ws(w), w.id));
        if json {
            let rows: Vec<_> = windows
                .iter()
                .map(|w| {
                    let f = w.frame;
                    json!({
                        "workspace": ws(w), "id": w.id, "pid": w.app.pid,
                        "app": w.app.bundle_id, "title": w.title,
                        "frame": { "x": f.x, "y": f.y, "w": f.w, "h": f.h },
                        "parked": self.parked.contains_key(&w.id),
                    })
                })
                .collect();
            return Ok(serde_json::Value::from(rows).to_string());
        }
        let focused = self.spaces.focused() + 1;
        let line = |w: &WinInfo| {
            let n = ws(w);
            let mark = if n == Some(focused) { "*" } else { " " };
            let n = n.map_or_else(|| "-".to_string(), |n| n.to_string());
            let app = w.app.bundle_id.clone().unwrap_or_else(|| format!("pid {}", w.app.pid));
            let f = w.frame;
            let parked = if self.parked.contains_key(&w.id) { " parked" } else { "" };
            format!(
                "{n}{mark} {:>6}  {app}  {:.0},{:.0} {:.0}x{:.0}{parked}  {}",
                w.id, f.x, f.y, f.w, f.h, w.title
            )
        };
        let lines: Vec<String> = windows.iter().map(line).collect();
        Ok(if lines.is_empty() { "no windows".into() } else { lines.join("\n") })
    }
}

/// Windows that join workspaces: standard windows, not dialogs, panels or sheets.
fn managed(w: &WinInfo) -> bool {
    w.subrole.as_deref() == Some("AXStandardWindow")
}

/// Put every window `store` lists that winman knows back at its saved frame and wait at most
/// `wait` for its apps, then forget those rows. When an app does not answer in time
/// every row stays, for the next start. Runs at quit and when a reload turns tile off.
fn put_back(store: &Store, hooks: &Hooks, wait: Duration) {
    let known: HashSet<WinId> = (hooks.snapshot)().iter().map(|w| w.id).collect();
    let frames: Vec<(WinId, Rect)> = store
        .parked_all()
        .into_iter()
        .filter(|r| known.contains(&r.id))
        .map(|r| (r.id, r.frame))
        .collect();
    if frames.is_empty() {
        return;
    }
    if (hooks.place)(&frames).wait(wait).is_none() {
        (hooks.log)("an app did not answer; its windows go back at the next start");
        return;
    }
    store.parked_delete(&frames.iter().map(|&(id, _)| id).collect::<Vec<_>>());
}

impl Module for Tile {
    fn id(&self) -> &'static str {
        ID
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let settings: Settings = table.get()?;
        if !(1..=9).contains(&settings.workspaces) {
            return Err("[tile] workspaces: must be 1-9".into());
        }
        self.spaces.resize(settings.workspaces.into());
        Ok(())
    }

    fn on_event(&mut self, event: Event, cx: &mut Cx) -> bool {
        match event {
            Event::Started => self.start(cx),
            Event::ModuleChanged { module: ID } if self.running => {
                (self.hooks.drain)();
                self.sync(cx);
            }
            _ => {}
        }
        false
    }

    /// `--json` (`cx.json`) makes `ls` answer with a JSON array.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v] if v == "ls" => self.ls(cx.json),
            [v, n] if v == "ws" => self.switch(n, cx),
            [v] if v == "restore" => self.restore(cx),
            _ => Err(unknown_verb(ID, args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "tile ls [--json] | tile ws <n> | tile restore"
    }
}
