//! Module `flick`: the installed build compared with the local checkout it came from.
//! Ids are `flick:<key>`: `flick:version`, `flick:rebuild-available`.
//!
//! Table `[flick]`: `source` (the checkout; default the baked `FLICK_BUILD_SOURCE`, `~`
//! expanded) and `check_on_open` (default true). Checks run git on a thread on
//! `LauncherOpened` and `Wake`, at most once per `CHECK_EVERY`; `items` reads the cached
//! result, so the main thread never waits on git. Never rebuilds by itself.

mod git;
mod stamp;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::config::Section;
use crate::core::{Cx, Event, Icon, Item, ItemId, Module, Outcome, unknown_verb};
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings { source: None, check_on_open: true }
    }
}

/// The last check's result, with the checkout it ran in.
type Cache = Arc<Mutex<Option<(PathBuf, Result<Status, String>)>>>;

pub struct Rebuild {
    stamp: Stamp,
    source: PathBuf,
    check_on_open: bool,
    cache: Cache,
    last_check: Option<Instant>,
}

impl Default for Rebuild {
    fn default() -> Self {
        Rebuild::new(Stamp::baked())
    }
}

impl Rebuild {
    fn new(stamp: Stamp) -> Rebuild {
        let source = stamp.source.clone();
        Rebuild { stamp, source, check_on_open: true, cache: Cache::default(), last_check: None }
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
        let _ = std::thread::Builder::new().name("flick-git".into()).spawn(move || {
            let status = git::check(&source, installed.as_deref(), CHECK_BUDGET);
            Rebuild::store(&cache, source, status);
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
        Ok(())
    }

    /// `Flick Version` always; `Rebuild Available` once a check found a HEAD other than
    /// the installed sha.
    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        let status = self.status();
        let mut items = vec![self.version_item(status.as_ref())];
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

    fn activate(&mut self, id: &ItemId, _cx: &mut Cx) -> Outcome {
        match id.key() {
            "version" => Outcome::Stay(Some(format!("Flick {} {}", env!("CARGO_PKG_VERSION"), self.stamp.label()))),
            // TODO(flick-31c8): build in the background instead of pointing at the script.
            "rebuild-available" => {
                Outcome::Stay(Some(format!("Run scripts/bundle.sh --install in {}", tilde(&self.source))))
            }
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
            _ => Err(unknown_verb("flick", args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "flick version"
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
