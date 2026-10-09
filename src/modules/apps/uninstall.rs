//! Uninstall: move an app bundle and its leftovers to the Trash, after a confirmation that
//! lists exactly what will move. The trash function is a parameter (`platform::files::trash`
//! in the app), so nothing here removes a file any other way and tests never touch the Trash.

use std::path::{Path, PathBuf};

use super::Apps;
use super::leftovers::{self, Size};
use crate::core::{Confirm, ConfirmRow, Outcome};
use crate::platform::workspace;

/// Prefix of the confirm token; the app's path follows it.
const TOKEN: &str = "uninstall/";

/// Exactly what an uninstall moves: the app bundle first, then its leftovers, with sizes.
#[derive(Debug)]
pub(super) struct Plan {
    name: String,
    bid: String,
    paths: Vec<(PathBuf, Size)>,
}

impl Plan {
    fn app(&self) -> &Path {
        &self.paths[0].0
    }

    fn token(&self) -> String {
        format!("{TOKEN}{}", self.app().display())
    }

    /// "3 items (1.2 MB)".
    fn amount(&self, paths: &[PathBuf]) -> String {
        let mut total = Size::default();
        for (_, size) in self.paths.iter().filter(|(p, _)| paths.contains(p)) {
            total.bytes += size.bytes;
            total.capped |= size.capped;
        }
        let items = if paths.len() == 1 { "item" } else { "items" };
        format!("{} {items} ({})", paths.len(), human(total))
    }

    fn all(&self) -> Vec<PathBuf> {
        self.paths.iter().map(|(p, _)| p.clone()).collect()
    }

    fn confirm(&self, home: &Path) -> Confirm {
        let rows = self.paths.iter().enumerate().map(|(i, (path, size))| ConfirmRow {
            title: tilde(path, home),
            subtitle: if i == 0 { "Application".into() } else { String::new() },
            accessory: human(*size),
        });
        Confirm {
            module: "app",
            token: self.token(),
            title: format!("Uninstall {}", self.name),
            rows: rows.collect(),
            label: format!("Move {} to Trash", self.amount(&self.all())),
            destructive: true,
        }
    }

    /// `<size>\t<path>` per path, then `<total>\ttotal`.
    fn listing(&self) -> String {
        let lines = self.paths.iter().map(|(p, s)| format!("{}\t{}", human(*s), p.display()));
        let mut total = Size::default();
        for (_, s) in &self.paths {
            total.bytes += s.bytes;
            total.capped |= s.capped;
        }
        lines.chain([format!("{}\ttotal", human(total))]).collect::<Vec<_>>().join("\n")
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

impl Apps {
    /// cmd+K lists Uninstall… only for an app `plan` would not refuse as protected. The
    /// bundle id read is cheap after the first: `NSBundle` caches bundles by path.
    pub(super) fn uninstallable(&self, path: &Path) -> bool {
        let bid = workspace::bundle_id(path);
        leftovers::protected(path, bid.as_deref(), self.own.as_deref(), &self.home).is_none()
    }

    /// The plan for uninstalling the app at `path`, or why it may not be: protected (macOS,
    /// Apple, Flick, outside the app folders), missing, or running.
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
        let mut paths = vec![(path.to_path_buf(), leftovers::size(path))];
        paths.extend(leftovers::find(&self.home, &bid).into_iter().map(|l| (l.path, l.size)));
        Ok(Plan { name, bid, paths })
    }

    /// Uninstall… on the app at `path`: the confirmation listing every path, kept as pending.
    pub(super) fn ask_uninstall(&mut self, path: &Path) -> Outcome {
        match self.plan(path) {
            Ok(plan) => {
                let confirm = plan.confirm(&self.home);
                self.pending = Some(plan);
                Outcome::Confirm(confirm)
            }
            Err(e) => Outcome::Stay(Some(e)),
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
            return Outcome::Stay(Some("Nothing to uninstall; choose Uninstall… again".into()));
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
        for (path, _) in &plan.paths[1..] {
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
    /// so nothing moves unless the caller says `--yes`.
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
        if dry {
            Ok(plan.listing())
        } else if yes {
            let report = self.execute(&plan, trash).map_err(|e| format!("app: {e}"))?;
            Ok(report.lines(&plan))
        } else {
            Err(format!("{}\napp: pass --yes to move these to the Trash", plan.listing()))
        }
    }
}

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
