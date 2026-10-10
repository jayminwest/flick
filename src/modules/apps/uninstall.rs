//! Uninstall: move an app bundle and its leftovers to the Trash, after a confirmation that
//! lists exactly what will move. The trash function is a parameter (`platform::files::trash`
//! in the app), so nothing here removes a file any other way and tests never touch the Trash.
//!
//! Sizing walks every planned path (up to 200,000 entries each), which takes seconds for a
//! big app, so it never runs on the main thread: a plan starts with no sizes and a sizing
//! thread fills them in. Uninstall… pushes view `app/uninstall`, which lists the paths with
//! "Sizing…" until the thread posts `ModuleChanged`; Enter there asks for the confirmation.
//! `app uninstall <name> --dry-run` answers later (`core::later`) with the sized listing.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use super::Apps;
use super::leftovers::{self, Size};
use crate::core::{Confirm, ConfirmRow, Icon, Item, ItemId, ListView, Outcome, later};
use crate::platform::workspace;

/// Prefix of the confirm token; the app's path follows it.
const TOKEN: &str = "uninstall/";
/// The review view's name, and the arg its items carry.
pub(super) const VIEW: &str = "uninstall";
pub(super) const ARG: &str = "uninstall";
/// A size the sizing thread has not reported yet.
const SIZING: &str = "Sizing…";
const NOTHING: &str = "Nothing to uninstall; choose Uninstall… again";

/// Sizes of a plan's paths, in order, once its sizing thread has them.
type Sizes = Arc<Mutex<Option<Vec<Size>>>>;

/// Exactly what an uninstall moves: the app bundle first, then its leftovers. Clones share
/// the sizes.
#[derive(Clone, Debug)]
pub(super) struct Plan {
    name: String,
    bid: String,
    paths: Vec<PathBuf>,
    sizes: Sizes,
}

impl Plan {
    fn app(&self) -> &Path {
        &self.paths[0]
    }

    fn token(&self) -> String {
        format!("{TOKEN}{}", self.app().display())
    }

    /// Each path with its size, `None` until the sizing thread is done.
    fn sized(&self) -> Vec<(&Path, Option<Size>)> {
        let sizes = self.sizes.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let size = |i: usize| sizes.as_ref().and_then(|s| s.get(i).copied());
        self.paths.iter().enumerate().map(|(i, p)| (p.as_path(), size(i))).collect()
    }

    /// Size every path on a thread, store the sizes, then call `done` on that thread. A
    /// thread that cannot start leaves the sizes unknown and drops `done`.
    fn start_sizing(&self, done: impl FnOnce() + Send + 'static) {
        let (paths, sizes) = (self.paths.clone(), Arc::clone(&self.sizes));
        let job = move || {
            let all = paths.iter().map(|p| leftovers::size(p)).collect();
            *sizes.lock().unwrap_or_else(PoisonError::into_inner) = Some(all);
            done();
        };
        let _ = thread::Builder::new().name("app-uninstall-size".into()).spawn(job);
    }

    /// The total size of the paths `keep` passes; `None` while any of them is unknown.
    fn total(&self, keep: impl Fn(&Path) -> bool) -> Option<Size> {
        let mut total = Size::default();
        for (_, size) in self.sized().into_iter().filter(|(p, _)| keep(p)) {
            let size = size?;
            total.bytes += size.bytes;
            total.capped |= size.capped;
        }
        Some(total)
    }

    /// "3 items (1.2 MB)", or "3 items" while sizing.
    fn amount(&self, paths: &[PathBuf]) -> String {
        let items = if paths.len() == 1 { "item" } else { "items" };
        let total = self.total(|p| paths.iter().any(|q| q == p));
        let size = total.map(|t| format!(" ({})", human(t))).unwrap_or_default();
        format!("{} {items}{size}", paths.len())
    }

    fn confirm(&self, home: &Path) -> Confirm {
        let rows = self.sized().into_iter().enumerate().map(|(i, (path, size))| ConfirmRow {
            title: tilde(path, home),
            subtitle: if i == 0 { "Application".into() } else { String::new() },
            accessory: shown(size),
        });
        Confirm {
            module: "app",
            token: self.token(),
            title: format!("Uninstall {}", self.name),
            rows: rows.collect(),
            label: format!("Move {} to Trash", self.amount(&self.paths)),
            destructive: true,
        }
    }

    /// `<size>\t<path>` per path, then `<total>\ttotal`.
    fn listing(&self) -> String {
        let lines = self.sized().into_iter().map(|(p, s)| format!("{}\t{}", shown(s), p.display()));
        let total = shown(self.total(|_| true));
        lines.chain([format!("{total}\ttotal")]).collect::<Vec<_>>().join("\n")
    }
}

/// What an uninstall did with each planned path.
#[derive(Debug, Default)]
pub(super) struct Report {
    moved: Vec<PathBuf>,
    /// No longer a match when the user confirmed (gone, replaced, now a symlink).
    skipped: Vec<PathBuf>,
    failed: Vec<(PathBuf, String)>,
}

impl Report {
    /// The status line, e.g. "Moved 4 items (1.2 MB) to Trash".
    fn status(&self, plan: &Plan) -> String {
        let mut parts = vec![format!("Moved {} to Trash", plan.amount(&self.moved))];
        if !self.skipped.is_empty() {
            parts.push(format!("skipped {} that changed", self.skipped.len()));
        }
        if let Some((path, err)) = self.failed.first() {
            let n = self.failed.len();
            parts.push(format!("{n} could not move ({}: {err})", path.display()));
        }
        parts.join("; ")
    }

    /// One `moved|skipped|failed\t<path>[\t<error>]` line per path, then the status.
    fn lines(&self, plan: &Plan) -> String {
        let moved = self.moved.iter().map(|p| format!("moved\t{}", p.display()));
        let skipped = self.skipped.iter().map(|p| format!("skipped\t{}", p.display()));
        let failed = self.failed.iter().map(|(p, e)| format!("failed\t{}\t{e}", p.display()));
        let status = [self.status(plan)];
        moved.chain(skipped).chain(failed).chain(status).collect::<Vec<_>>().join("\n")
    }
}

/// View `app/uninstall`, empty until `refresh_uninstall` fills it.
pub(super) fn view() -> ListView {
    ListView { empty: "Nothing to uninstall".into(), ..ListView::new("app", VIEW) }
}

impl Apps {
    /// cmd+K lists Uninstall… only for an app `plan` would not refuse as protected. The
    /// bundle id read is cheap after the first: `NSBundle` caches bundles by path.
    pub(super) fn uninstallable(&self, path: &Path) -> bool {
        let bid = workspace::bundle_id(path);
        leftovers::protected(path, bid.as_deref(), self.own.as_deref(), &self.home).is_none()
    }

    /// The plan for uninstalling the app at `path`, or why it may not be: protected (macOS,
    /// Apple, Flick, outside the app folders), missing, or running. Its sizes are unknown
    /// until `Plan::start_sizing` finishes; finding the paths lists only a few folders.
    pub(super) fn plan(&self, path: &Path) -> Result<Plan, String> {
        let name = self.name_of(path);
        let bid = workspace::bundle_id(path);
        let own = self.own.as_deref();
        // A relative or empty home would search `Library` under the working directory.
        if !self.home.is_absolute() {
            return Err(format!("Cannot uninstall {name}: no home folder"));
        }
        if let Some(why) = leftovers::protected(path, bid.as_deref(), own, &self.home) {
            return Err(format!("Cannot uninstall {name}: {why}"));
        }
        if !real_dir(path) {
            return Err(format!("{name} is no longer at {}", path.display()));
        }
        if !workspace::running_for_bundle(path).is_empty() {
            return Err(format!("Quit {name} first"));
        }
        let bid = bid.unwrap_or_default();
        let mut paths = vec![path.to_path_buf()];
        paths.extend(leftovers::matches(&self.home, &bid));
        Ok(Plan { name, bid, paths, sizes: Sizes::default() })
    }

    /// Uninstall… on the app at `path`: keep its plan as pending, start sizing it (`notify`
    /// runs on the sizing thread when it is done) and push the review view.
    pub(super) fn ask_uninstall(&mut self, path: &Path, notify: fn()) -> Outcome {
        match self.plan(path) {
            Ok(plan) => {
                plan.start_sizing(notify);
                self.pending = Some(plan);
                Outcome::Push(view())
            }
            Err(e) => Outcome::Stay(Some(e)),
        }
    }

    /// The review view: one row per pending path with its size, and the total in the footer.
    /// It reads the sizes without waiting; the sizing thread's `ModuleChanged` refreshes it.
    pub(super) fn refresh_uninstall(&self, view: &mut ListView) {
        let Some(plan) = &self.pending else {
            view.items.clear();
            view.footer.clear();
            return;
        };
        view.footer = format!("Enter to move {} to Trash…", plan.amount(&plan.paths));
        let rows = plan.sized().into_iter().enumerate().map(|(i, (path, size))| Item {
            subtitle: if i == 0 { "Application".into() } else { String::new() },
            accessory: shown(size),
            ..Item::new(
                ItemId::new("app", path.display()).with_arg(ARG),
                tilde(path, &self.home),
                "Move to Trash",
                Icon::File(path.to_path_buf()),
            )
        });
        view.items = rows.collect();
    }

    /// Enter in the review view: the confirmation listing every pending path.
    pub(super) fn confirm_pending(&self) -> Outcome {
        match &self.pending {
            Some(plan) => Outcome::Confirm(plan.confirm(&self.home)),
            None => Outcome::Stay(Some(NOTHING.into())),
        }
    }

    /// The user confirmed `token`: move exactly the pending plan's paths with `trash`.
    pub(super) fn confirm_uninstall(
        &mut self,
        token: &str,
        trash: impl FnMut(&Path) -> Result<PathBuf, String>,
    ) -> Outcome {
        let plan = self.pending.take_if(|p| p.token() == token);
        let Some(plan) = plan else {
            return Outcome::Stay(Some(NOTHING.into()));
        };
        let status = self.execute(&plan, trash).map_or_else(|e| e, |r| r.status(&plan));
        Outcome::Stay(Some(status))
    }

    /// Move `plan`'s paths to the Trash, the bundle first. Each path is checked again first:
    /// the app must still be the same unprotected, stopped bundle (else nothing moves), and a
    /// leftover must still be a match (else it is skipped). A failed leftover does not stop
    /// the others. `Err` when nothing moved.
    fn execute(
        &mut self,
        plan: &Plan,
        mut trash: impl FnMut(&Path) -> Result<PathBuf, String>,
    ) -> Result<Report, String> {
        let app = plan.app();
        let fresh = self.plan(app)?;
        if fresh.bid != plan.bid {
            return Err(format!("{} changed; nothing was moved", plan.name));
        }
        trash(app).map_err(|e| {
            format!("Could not move {} to the Trash: {e}; nothing was moved", plan.name)
        })?;
        self.apps.retain(|a| a.path != app);
        let mut report = Report { moved: vec![app.to_path_buf()], ..Report::default() };
        for path in &plan.paths[1..] {
            if !leftovers::still_matches(&self.home, &plan.bid, path) {
                report.skipped.push(path.clone());
                continue;
            }
            match trash(path) {
                Ok(_) => report.moved.push(path.clone()),
                Err(e) => report.failed.push((path.clone(), e)),
            }
        }
        Ok(report)
    }

    /// `app uninstall <name> --dry-run|--yes`. Without a flag it lists the paths and fails,
    /// so nothing moves unless the caller says `--yes`. A listing answers later, from the
    /// sizing thread; the immediate answer (for callers that do not wait) has no sizes.
    /// `--yes` moves without sizing, so its status counts items only.
    pub(super) fn uninstall_command(
        &mut self,
        args: &[String],
        trash: impl FnMut(&Path) -> Result<PathBuf, String>,
    ) -> Result<String, String> {
        let (flags, words): (Vec<&String>, Vec<&String>) =
            args.iter().partition(|a| a.starts_with("--"));
        let usage = "app: usage: app uninstall <name> [--dry-run | --yes]";
        let (dry, yes) = match flags.as_slice() {
            [] => (false, false),
            [f] if *f == "--dry-run" => (true, false),
            [f] if *f == "--yes" => (false, true),
            _ => return Err(usage.into()),
        };
        if words.is_empty() {
            return Err(usage.into());
        }
        let name = words.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(" ");
        let path = self.named(&name)?.path.clone();
        let plan = self.plan(&path).map_err(|e| format!("app: {e}"))?;
        if yes {
            let report = self.execute(&plan, trash).map_err(|e| format!("app: {e}"))?;
            return Ok(report.lines(&plan));
        }
        let (tx, sized) = (later::answer_later(), plan.clone());
        plan.start_sizing(move || {
            let listing = sized.listing();
            let answer = if dry { Ok(listing) } else { Err(format!("{listing}\n{PASS_YES}")) };
            let _ = tx.send(answer);
        });
        Ok(plan.listing())
    }
}

const PASS_YES: &str = "app: pass --yes to move these to the Trash";

/// A real folder (an app bundle), not a symlink to one.
fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// `path` with the home folder shown as `~`.
fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// `human`, or "Sizing…" for a size not known yet.
fn shown(size: Option<Size>) -> String {
    size.map_or_else(|| SIZING.into(), human)
}

/// "512 bytes", "1.2 MB" (powers of 1000, like Finder); "≥ " first for a capped size.
fn human(size: Size) -> String {
    let at_least = if size.capped { "≥ " } else { "" };
    let units = ["KB", "MB", "GB", "TB"];
    if size.bytes < 1000 {
        let s = if size.bytes == 1 { "" } else { "s" };
        return format!("{at_least}{} byte{s}", size.bytes);
    }
    let mut value = size.bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{at_least}{value:.1} {}", units[unit])
}

#[cfg(test)]
mod tests;
