//! Module `flick`: the installed build compared with the local checkout it came from, and
//! rebuilding from it. Ids are `flick:<key>`: `flick:version`, `flick:rebuild`,
//! `flick:rebuild-dirty`, `flick:rebuild-available`; in view `flick/build`,
//! `flick:build-status`, `flick:cancel-build` and `flick:build-log`.
//!
//! Table `[flick]`: `source` (the checkout; default the baked `FLICK_BUILD_SOURCE`, `~`
//! expanded), `check_on_open` (default true) and `gates` (run check-all.sh before bundling;
//! default false). Checks run git on a thread on `LauncherOpened` and `Wake`, at most once
//! per `CHECK_EVERY`; `items` reads the cached result, so the main thread never waits on
//! git. Never rebuilds by itself: builds start from an item or `flick flick rebuild`, run on
//! the runner's thread (`build.rs`), and post `ModuleChanged` as they progress.

mod build;
mod git;
mod stamp;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::config::Section;
use crate::core::{Cx, Event, Icon, Item, ItemId, ListView, Module, Outcome, unknown_verb};
use crate::platform::events;
use build::{Hooks, Paths, Request, Runner};
use git::Status;
use stamp::Stamp;

/// Background checks start at most this often.
const CHECK_EVERY: Duration = Duration::from_secs(30);
/// Time for all git calls of one background check.
const CHECK_BUDGET: Duration = Duration::from_secs(10);
/// Time for `flick flick version`, which runs git on the main thread.
const COMMAND_BUDGET: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    source: Option<String>,
    check_on_open: bool,
    gates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { source: None, check_on_open: true, gates: false }
    }
}

/// Progress and finished checks show at once in a visible view; a failed build opens its
/// log in the default text editor.
const HOOKS: Hooks = Hooks {
    notify: || events::post(Event::ModuleChanged { module: "flick" }),
    open_log: |log| {
        let _ = std::process::Command::new("open").arg("-t").arg(log).status();
    },
};

/// The last check's result, with the checkout it ran in.
type Cache = Arc<Mutex<Option<(PathBuf, Result<Status, String>)>>>;

pub struct Rebuild {
    stamp: Stamp,
    source: PathBuf,
    check_on_open: bool,
    gates: bool,
    cache: Cache,
    last_check: Option<Instant>,
    runner: Runner,
    /// Restart Flick after an install; tests turn it off.
    restart: bool,
    /// relaunch.sh's install dir; `None` is its default, ~/Applications.
    install_dir: Option<PathBuf>,
}

impl Default for Rebuild {
    fn default() -> Self {
        Rebuild::new(Stamp::baked())
    }
}

impl Rebuild {
    fn new(stamp: Stamp) -> Rebuild {
        let source = stamp.source.clone();
        let runner = Runner::new(Paths::standard(), HOOKS);
        Rebuild {
            stamp,
            source,
            check_on_open: true,
            gates: false,
            cache: Cache::default(),
            last_check: None,
            runner,
            restart: true,
            install_dir: None,
        }
    }

    /// The cached status of the configured checkout; `None` before its first check.
    fn status(&self) -> Option<Result<Status, String>> {
        let cache = self.cache.lock().ok()?;
        cache.as_ref().filter(|(source, _)| *source == self.source).map(|(_, s)| s.clone())
    }

    fn store(cache: &Cache, source: PathBuf, status: Result<Status, String>) {
        if let Ok(mut c) = cache.lock() {
            *c = Some((source, status));
        }
    }

    /// Check the checkout on a thread; the result lands in the cache.
    fn spawn_check(&self) {
        let (cache, source, installed) = (self.cache.clone(), self.source.clone(), self.stamp.sha.clone());
        let notify = self.runner.hooks.notify;
        let _ = std::thread::Builder::new().name("flick-git".into()).spawn(move || {
            let status = git::check(&source, installed.as_deref(), CHECK_BUDGET);
            Rebuild::store(&cache, source, status);
            notify();
        });
    }

    /// The checkout part of the version subtitle.
    fn checkout_note(&self, status: &Result<Status, String>) -> String {
        let source = tilde(&self.source);
        match status {
            Err(e) => format!("checkout {source}: {e}"),
            Ok(s) if self.stamp.sha.as_deref() == Some(&s.head) => {
                let dirty = if s.dirty { " (uncommitted changes)" } else { "" };
                format!("up to date with {source}{dirty}")
            }
            Ok(Status { newer: Some(n @ 1..), subjects, .. }) => {
                let commits = if *n == 1 { "commit" } else { "commits" };
                let subjects = subjects.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>();
                format!("{n} {commits} newer in {source}: {}", subjects.join("; "))
            }
            Ok(s) => format!("{source} is at {}", &s.head[..7]),
        }
    }

    /// A build of `rev` (`None`: the tree as it is) of checkout `source`. The installed app
    /// keeps tracking this Flick's checkout (`home`), whichever tree it was built from.
    fn request(&self, rev: Option<String>, source: PathBuf, open_log: bool) -> Request {
        let (home, gates, restart) = (self.source.clone(), self.gates, self.restart);
        let install_dir = self.install_dir.clone();
        Request { source, home, rev, gates, restart, install_dir, open_log }
    }

    /// Start a build and show its progress; a build already running shows instead.
    fn start(&mut self, rev: Option<&str>) -> Outcome {
        let req = self.request(rev.map(String::from), self.source.clone(), true);
        match self.runner.start(req) {
            Err(e) if !self.runner.progress().active() => Outcome::Stay(Some(e)),
            _ => Outcome::Push(ListView::new("flick", "build")),
        }
    }

    /// `Rebuild Flick` and `Rebuild Flick (Dirty)`; while a build runs, its progress.
    fn rebuild_items(&self) -> Vec<Item> {
        let progress = self.runner.progress();
        let busy = progress.active().then(|| progress.title(Instant::now()));
        let source = tilde(&self.source);
        let rows = [
            ("rebuild", "Rebuild Flick", format!("Build HEAD of {source}, install and restart")),
            (
                "rebuild-dirty",
                "Rebuild Flick (Dirty)",
                format!("Build {source} with uncommitted changes, install and restart"),
            ),
        ];
        rows.into_iter()
            .map(|(key, title, subtitle)| Item {
                subtitle: busy.clone().unwrap_or(subtitle),
                accessory: "Flick".into(),
                keywords: vec!["update install build compile restart".into()],
                ..Item::new(ItemId::new("flick", key), title, "Run Command", Icon::Symbol("hammer"))
            })
            .collect()
    }

    /// View `flick/build`: where the build is, Cancel while it runs, and the log.
    fn build_items(&self) -> Vec<Item> {
        let p = self.runner.progress();
        let status = Item {
            subtitle: p.last_line.clone(),
            tone: p.tone(),
            ..Item::new(ItemId::new("flick", "build-status"), p.title(Instant::now()), "Open Log", Icon::Symbol("hammer"))
        };
        let mut items = vec![status];
        if p.phase == build::Phase::Building {
            items.push(Item::new(ItemId::new("flick", "cancel-build"), "Cancel Build", "Run Command", Icon::Symbol("xmark.circle")));
        }
        items.push(Item {
            subtitle: tilde(&self.runner.paths.log),
            ..Item::new(ItemId::new("flick", "build-log"), "Open Build Log", "Open", Icon::Symbol("doc.text"))
        });
        items
    }

    /// Show the build log in the default text editor.
    fn open_log(&self, cx: &mut Cx) -> Outcome {
        let log = &self.runner.paths.log;
        if !log.exists() {
            return Outcome::Stay(Some("No build log yet".into()));
        }
        cx.hide();
        let _ = std::process::Command::new("open").arg("-t").arg(log).spawn();
        Outcome::Hide
    }

    fn version_item(&self, status: Option<&Result<Status, String>>) -> Item {
        let mut subtitle = self.stamp.label();
        if let Some(status) = status {
            subtitle = format!("{subtitle} · {}", self.checkout_note(status));
        }
        Item {
            subtitle,
            accessory: "Flick".into(),
            keywords: vec!["about build commit sha update".into()],
            ..Item::new(ItemId::new("flick", "version"), "Flick Version", "Show Version", Icon::Symbol("info.circle"))
        }
    }

    /// `flick flick version`: the installed stamp and the checkout, one fact per line.
    fn version_text(&self, status: &Result<Status, String>) -> String {
        let clean = |dirty: bool| if dirty { "dirty" } else { "clean" };
        let installed = self.stamp.sha.as_deref().unwrap_or("dev");
        let built = self.stamp.time.as_deref().unwrap_or("unknown");
        let mut lines = vec![
            format!("version   {}", env!("CARGO_PKG_VERSION")),
            format!("installed {installed} {} built {built}", clean(self.stamp.dirty)),
            format!("source    {}", self.source.display()),
        ];
        match status {
            Err(e) => lines.push(format!("checkout  error: {e}")),
            Ok(s) => {
                let newer = match s.newer {
                    Some(n) => format!(" {n} newer"),
                    None => String::new(),
                };
                lines.push(format!("checkout  {} {}{newer}", s.head, clean(s.dirty)));
                lines.extend(s.subjects.iter().map(|(sha, subject)| format!("  {sha} {subject}")));
            }
        }
        lines.join("\n")
    }
}

impl Module for Rebuild {
    fn id(&self) -> &'static str {
        "flick"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let settings = table.get::<Settings>()?;
        let source = settings.source.map_or_else(|| self.stamp.source.clone(), |s| expand(&s));
        if source != self.source {
            // The next open checks the new checkout at once.
            self.last_check = None;
        }
        self.source = source;
        self.check_on_open = settings.check_on_open;
        self.gates = settings.gates;
        Ok(())
    }

    /// `Flick Version` and the rebuild items always; `Rebuild Available` once a check found
    /// a HEAD other than the installed sha.
    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        let status = self.status();
        let mut items = vec![self.version_item(status.as_ref())];
        items.extend(self.rebuild_items());
        if let Some(Ok(s)) = &status
            && self.stamp.sha.as_deref() != Some(&s.head)
        {
            items.push(Item {
                subtitle: format!("Build {} from {}", &s.head[..7], tilde(&self.source)),
                accessory: "Flick".into(),
                keywords: vec!["update upgrade install".into()],
                ..Item::new(
                    ItemId::new("flick", "rebuild-available"),
                    "Rebuild Available",
                    "Run Command",
                    Icon::Symbol("arrow.triangle.2.circlepath"),
                )
            });
        }
        items
    }

    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        (view == "build").then(|| ListView {
            placeholder: "Flick Build".into(),
            footer: "Flick Build  ·  esc to go back".into(),
            ..ListView::new("flick", view)
        })
    }

    fn refresh(&mut self, view: &mut ListView, _cx: &mut Cx) {
        view.items = self.build_items();
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        match id.key() {
            "version" => Outcome::Stay(Some(format!("Flick {} {}", env!("CARGO_PKG_VERSION"), self.stamp.label()))),
            "rebuild" | "rebuild-available" => self.start(Some("HEAD")),
            "rebuild-dirty" => self.start(None),
            "cancel-build" => Outcome::Stay(Some(self.runner.cancel())),
            "build-log" | "build-status" => self.open_log(cx),
            _ => Outcome::Stay(None),
        }
    }

    /// Starts a background check; the cache, not this call, carries the result.
    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        let now = Instant::now();
        if self.check_on_open
            && matches!(event, Event::LauncherOpened | Event::Wake)
            && self.last_check.is_none_or(|t| now.duration_since(t) >= CHECK_EVERY)
        {
            self.last_check = Some(now);
            self.spawn_check();
        }
        false
    }

    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        match args {
            [verb] if verb == "version" => {
                let status = git::check(&self.source, self.stamp.sha.as_deref(), COMMAND_BUDGET);
                let text = self.version_text(&status);
                Rebuild::store(&self.cache, self.source.clone(), status);
                Ok(text)
            }
            [verb, rest @ ..] if verb == "rebuild" => {
                let (rev, source) = parse_rebuild(rest, &self.source)?;
                self.runner.start(self.request(rev, source, false))
            }
            [verb] if verb == "status" => {
                let p = self.runner.progress();
                let mut lines = vec![p.summary(Instant::now())];
                lines.extend((!p.last_line.is_empty()).then(|| format!("  {}", p.last_line)));
                lines.push(format!("log: {}", self.runner.paths.log.display()));
                Ok(lines.join("\n"))
            }
            [verb] if verb == "cancel" => Ok(self.runner.cancel()),
            _ => Err(unknown_verb("flick", args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "flick version | flick rebuild [--dirty] [--ref <rev>] [--source <dir>] | flick status | flick cancel"
    }
}

/// `flick flick rebuild` options as (rev, checkout): `--ref <rev>` (default `HEAD`),
/// `--dirty` (the tree as it is; no rev), `--source <dir>` (default `source`).
fn parse_rebuild(args: &[String], source: &Path) -> Result<(Option<String>, PathBuf), String> {
    let (mut rev, mut dirty, mut dir) = (None, false, source.to_path_buf());
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dirty" => dirty = true,
            "--ref" => rev = Some(args.next().ok_or("flick rebuild: --ref needs a rev")?.clone()),
            "--source" => dir = expand(args.next().ok_or("flick rebuild: --source needs a directory")?),
            other => return Err(format!("flick rebuild: unknown option {other:?}")),
        }
    }
    match (dirty, rev) {
        (true, Some(_)) => Err("flick rebuild: --dirty builds the tree as it is; drop --ref".into()),
        (true, None) => Ok((None, dir)),
        (false, rev) => Ok((Some(rev.unwrap_or_else(|| "HEAD".into())), dir)),
    }
}

/// `~` and `~/...` under the home directory.
fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix('~'), dirs::home_dir()) {
        (Some(""), Some(home)) => home,
        (Some(rest), Some(home)) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

/// `path` with the home directory shown as `~`.
fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests;
