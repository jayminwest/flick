//! The rebuild runner: builds Flick from the checkout on a thread, one build at a time, and
//! hands the bundle to `scripts/relaunch.sh`.
//!
//! A rev build (`HEAD` by default) exports that commit with `git archive` first, so
//! uncommitted edits stay out; a dirty build runs in the checkout as it is. Both run
//! `scripts/bundle.sh` through the login shell (launchd starts Flick with a bare PATH, so
//! cargo is not on it) with `CARGO_TARGET_DIR=<source>/target/flick-rebuild` and
//! `--locked --offline`. All output goes to the log. The build and the installer each run in
//! their own process group: cancel stops cargo and rustc with the build, and the installer
//! outlives this process when it restarts Flick.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::git;
use crate::core::Tone;

/// Time for `git rev-parse` and `git archive` together.
const GIT_BUDGET: Duration = Duration::from_secs(120);
/// How often the build thread checks its child.
const POLL: Duration = Duration::from_millis(100);
/// Progress goes out every this many polls (once a second), and only while a build runs.
const TICKS: u32 = 10;
/// Cargo flags for every rebuild: the checkout's lock file, no network.
pub const CARGO_ARGS: &str = "--locked --offline";
/// Longest last-log-line the build view shows.
const LINE_MAX: usize = 160;

/// What to build, and what to do with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The checkout (or another worktree) to build.
    pub source: PathBuf,
    /// The checkout baked as `FLICK_BUILD_SOURCE`, the installed app's default `[flick]
    /// source`: the one this Flick tracks, not a `--source` worktree, so its later checks
    /// still compare against that checkout (flick-be14).
    pub home: PathBuf,
    /// The local rev to export (e.g. `HEAD`); `None` builds the tree as it is.
    pub rev: Option<String>,
    /// Run `scripts/check-all.sh --bail` before bundling.
    pub gates: bool,
    /// Restart Flick after install (relaunch.sh `--wait-pid <this pid>`); else
    /// `--no-restart`.
    pub restart: bool,
    /// `FLICK_INSTALL_DIR` for relaunch.sh; `None` keeps its default.
    pub install_dir: Option<PathBuf>,
    /// Open the log in a text editor when the build fails.
    pub open_log: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Building,
    Installing,
    Installed,
    Failed(String),
    Cancelled,
}

/// The current or last build.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub phase: Phase,
    /// `abc1234`, `abc1234-dirty`, or what was asked for until git resolves it.
    pub label: String,
    pub started: Option<Instant>,
    pub ended: Option<Instant>,
    /// The last line of the log.
    pub last_line: String,
}

impl Progress {
    /// Building or installing.
    pub fn active(&self) -> bool {
        matches!(self.phase, Phase::Building | Phase::Installing)
    }

    /// Seconds from start to end, or to `now` while it runs.
    pub fn secs(&self, now: Instant) -> u64 {
        self.started.map_or(0, |s| self.ended.unwrap_or(now).saturating_duration_since(s).as_secs())
    }

    /// One line for `flick flick status`.
    pub fn summary(&self, now: Instant) -> String {
        let (label, secs) = (&self.label, self.secs(now));
        match &self.phase {
            Phase::Idle => "idle".into(),
            Phase::Building => format!("building {secs}s {label}"),
            Phase::Installing => format!("installing {label}"),
            Phase::Installed => format!("installed {label} after {secs}s"),
            Phase::Failed(e) => format!("failed {label}: {e}"),
            Phase::Cancelled => format!("cancelled {label} after {secs}s"),
        }
    }

    /// The title of the build view's status row.
    pub fn title(&self, now: Instant) -> String {
        let label = &self.label;
        match &self.phase {
            Phase::Idle => "No Build Running".into(),
            Phase::Building => format!("Building {label}… {}s", self.secs(now)),
            Phase::Installing => format!("Installing {label}…"),
            Phase::Installed => format!("Installed {label}"),
            Phase::Failed(_) => "Build Failed (see log)".into(),
            Phase::Cancelled => "Build Cancelled".into(),
        }
    }

    /// The status row's tint: green installed, red failed, else none.
    pub fn tone(&self) -> Tone {
        match self.phase {
            Phase::Installed => Tone::Ok,
            Phase::Failed(_) => Tone::Error,
            Phase::Idle | Phase::Building | Phase::Installing | Phase::Cancelled => Tone::Neutral,
        }
    }
}

/// What the runner calls from its thread. Tests pass no-ops.
#[derive(Clone, Copy)]
pub struct Hooks {
    /// Progress changed: post `ModuleChanged` so a visible view refreshes.
    pub notify: fn(),
    /// The build failed: show the log.
    pub open_log: fn(&Path),
}

/// Where the runner writes.
#[derive(Clone, Debug)]
pub struct Paths {
    /// The build log; the previous one is kept as `<log>.1`.
    pub log: PathBuf,
    /// Where a rev build exports its tree; emptied first.
    pub export: PathBuf,
}

impl Paths {
    /// `~/Library/Logs/Flick/rebuild.log` and `~/Library/Caches/Flick/rebuild/src`.
    pub fn standard() -> Paths {
        let lib = dirs::home_dir().unwrap_or_else(std::env::temp_dir).join("Library");
        Paths {
            log: lib.join("Logs/Flick/rebuild.log"),
            export: lib.join("Caches/Flick/rebuild/src"),
        }
    }
}

#[derive(Default)]
struct Shared {
    progress: Progress,
    /// The running child's process group.
    pgid: Option<u32>,
    cancel: bool,
}

/// One build at a time, on a thread; `progress` reads where it is.
pub struct Runner {
    shared: Arc<Mutex<Shared>>,
    pub paths: Paths,
    pub hooks: Hooks,
}

impl Runner {
    pub fn new(paths: Paths, hooks: Hooks) -> Runner {
        Runner { shared: Arc::default(), paths, hooks }
    }

    pub fn progress(&self) -> Progress {
        lock(&self.shared).progress.clone()
    }

    /// Start building `req` on a thread. `Err` when a build already runs.
    pub fn start(&self, req: Request) -> Result<String, String> {
        let now = Instant::now();
        let what = req.rev.clone().unwrap_or_else(|| "the dirty tree".into());
        {
            let mut s = lock(&self.shared);
            if s.progress.active() {
                return Err(format!("already building {} ({}s)", s.progress.label, s.progress.secs(now)));
            }
            let progress = Progress { phase: Phase::Building, label: what.clone(), started: Some(now), ..Progress::default() };
            *s = Shared { progress, ..Shared::default() };
        }
        let (shared, paths, hooks) = (self.shared.clone(), self.paths.clone(), self.hooks);
        let reply = format!("building {what} of {}; log: {}", req.source.display(), paths.log.display());
        let spawned = thread::Builder::new().name("flick-rebuild".into()).spawn(move || run(&shared, &paths, hooks, &req));
        match spawned {
            Ok(_) => Ok(reply),
            Err(e) => {
                let e = format!("cannot start the build thread: {e}");
                let mut s = lock(&self.shared);
                s.progress.phase = Phase::Failed(e.clone());
                s.progress.ended = Some(now);
                Err(e)
            }
        }
    }

    /// Stop a running build (not an install that has begun).
    pub fn cancel(&self) -> String {
        let mut s = lock(&self.shared);
        if s.progress.phase != Phase::Building {
            return "no build running".into();
        }
        s.cancel = true;
        if let Some(pgid) = s.pgid {
            kill_group(pgid);
        }
        format!("cancelling build {}", s.progress.label)
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The build thread: build, install, then record how it ended.
fn run(shared: &Mutex<Shared>, paths: &Paths, hooks: Hooks, req: &Request) {
    let result = open_log(&paths.log).and_then(|mut log| {
        let done = build(shared, paths, hooks, req, &log).and_then(|(dir, app)| install(shared, hooks, req, &dir, &app, &log));
        if let Err(e) = &done {
            let _ = writeln!(log, "flick rebuild: {e}");
        }
        done
    });
    let mut s = lock(shared);
    let cancelled = s.cancel;
    s.pgid = None;
    s.progress.ended = Some(Instant::now());
    s.progress.last_line = last_line(&paths.log);
    s.progress.phase = match result {
        Ok(()) => Phase::Installed,
        Err(_) if cancelled => Phase::Cancelled,
        Err(e) => Phase::Failed(e),
    };
    let failed = matches!(s.progress.phase, Phase::Failed(_));
    drop(s);
    if failed && req.open_log {
        (hooks.open_log)(&paths.log);
    }
    (hooks.notify)();
}

/// Resolve the rev, export it (or take the tree as it is) and run bundle.sh. Returns the
/// tree that was built and the bundle it made.
fn build(shared: &Mutex<Shared>, paths: &Paths, hooks: Hooks, req: &Request, mut log: &File) -> Result<(PathBuf, PathBuf), String> {
    let rev = req.rev.as_deref().unwrap_or("HEAD");
    if rev.starts_with('-') {
        return Err(format!("bad rev {rev:?}"));
    }
    let _ = writeln!(log, "flick rebuild: {} of {} (cargo {CARGO_ARGS})", req.rev.as_deref().unwrap_or("dirty tree"), req.source.display());
    let deadline = Instant::now() + GIT_BUDGET;
    let sha = git::git(&req.source, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")], deadline)?.trim().to_string();
    let short = sha.get(..7).unwrap_or(&sha).to_string();
    lock(shared).progress.label = if req.rev.is_some() { short } else { format!("{short}-dirty") };
    (hooks.notify)();

    let dir = match req.rev {
        Some(_) => {
            export(&req.source, &sha, &paths.export, deadline)?;
            paths.export.clone()
        }
        None => req.source.clone(),
    };
    let target = req.source.join("target/flick-rebuild");
    let script = if req.gates { "scripts/check-all.sh --bail && exec scripts/bundle.sh" } else { "exec scripts/bundle.sh" };
    let mut cmd = login_shell(script);
    cmd.current_dir(&dir)
        .env("CARGO_TARGET_DIR", &target)
        .env("FLICK_CARGO_ARGS", CARGO_ARGS)
        .env("FLICK_BUILD_SHA", &sha)
        .env("FLICK_BUILD_DIRTY", if req.rev.is_some() { "0" } else { "1" })
        .env("FLICK_BUILD_SOURCE", &req.home)
        .env_remove("FLICK_BUILD_TIME");
    let _ = writeln!(log, "$ cd {} && {script}", dir.display());
    run_child(shared, hooks, cmd, log, &paths.log).map_err(|e| explain(&e, &tail(&paths.log), &req.source))?;
    Ok((dir, target.join("Flick.app")))
}

/// `git archive <sha>` unpacked into `export`, emptied first.
fn export(source: &Path, sha: &str, export: &Path, deadline: Instant) -> Result<(), String> {
    let _ = fs::remove_dir_all(export);
    fs::create_dir_all(export).map_err(|e| format!("{}: {e}", export.display()))?;
    let tar = export.with_extension("tar");
    let tar_arg = tar.to_str().ok_or_else(|| format!("{}: not UTF-8", tar.display()))?;
    git::git(source, &["archive", "--format=tar", "-o", tar_arg, sha], deadline)?;
    let unpacked = Command::new("/usr/bin/tar").arg("-xf").arg(&tar).arg("-C").arg(export).stdin(Stdio::null()).status();
    let _ = fs::remove_file(&tar);
    match unpacked {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("tar: {s}")),
        Err(e) => Err(format!("tar: {e}")),
    }
}

/// Install `app` with the tree's relaunch.sh, which restarts Flick unless `req` says not to.
fn install(shared: &Mutex<Shared>, hooks: Hooks, req: &Request, dir: &Path, app: &Path, mut log: &File) -> Result<(), String> {
    if lock(shared).cancel {
        return Err("cancelled".into());
    }
    if !app.is_dir() {
        return Err(format!("bundle.sh made no bundle at {}", app.display()));
    }
    lock(shared).progress.phase = Phase::Installing;
    (hooks.notify)();
    let mut cmd = Command::new(dir.join("scripts/relaunch.sh"));
    cmd.arg("--install").arg(app);
    if req.restart {
        cmd.arg("--wait-pid").arg(std::process::id().to_string());
    } else {
        cmd.arg("--no-restart");
    }
    if let Some(d) = &req.install_dir {
        cmd.env("FLICK_INSTALL_DIR", d);
    }
    let _ = writeln!(log, "$ {cmd:?}");
    run_child(shared, hooks, cmd, log, Path::new("")).map_err(|e| format!("install {e}"))
}

/// `$SHELL -lc <script>` (zsh when unset): the user's PATH, with cargo on it.
fn login_shell(script: &str) -> Command {
    let shell = std::env::var_os("SHELL").filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
    let mut cmd = Command::new(shell);
    cmd.arg("-lc").arg(script);
    cmd
}

/// Run `cmd` in its own process group with output to `log`, until it exits. While it runs,
/// the last line of `log_path` (when given) goes to the progress once a second.
fn run_child(shared: &Mutex<Shared>, hooks: Hooks, mut cmd: Command, log: &File, log_path: &Path) -> Result<(), String> {
    let output = || log.try_clone().map(Stdio::from).map_err(|e| format!("log: {e}"));
    cmd.stdin(Stdio::null()).stdout(output()?).stderr(output()?).process_group(0);
    let name = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd.spawn().map_err(|e| format!("{name}: {e}"))?;
    {
        let mut s = lock(shared);
        s.pgid = Some(child.id());
        if s.cancel {
            kill_group(child.id());
        }
    }
    let mut ticks = 0u32;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(POLL),
            Err(e) => return Err(format!("{name}: {e}")),
        }
        ticks += 1;
        if ticks.is_multiple_of(TICKS) && !log_path.as_os_str().is_empty() {
            lock(shared).progress.last_line = last_line(log_path);
            (hooks.notify)();
        }
    };
    lock(shared).pgid = None;
    if status.success() { Ok(()) } else { Err(format!("failed ({status})")) }
}

/// SIGTERM to process group `pgid`. `/bin/kill` because std has no killpg.
fn kill_group(pgid: u32) {
    let _ = Command::new("/bin/kill").args(["-TERM", "--", &format!("-{pgid}")]).stderr(Stdio::null()).status();
}

/// A failed build's message, with a hint when the log tail shows a known cause.
fn explain(err: &str, tail: &str, source: &Path) -> String {
    if ["command not found: cargo", "cargo: command not found", "cargo not found"].iter().any(|s| tail.contains(s)) {
        "cargo not found on login-shell PATH: install Rust with rustup".into()
    } else if tail.contains("--offline was specified") || tail.contains("using offline mode") {
        format!("build {err}; a crate may be missing offline: run `cargo fetch` in {}", source.display())
    } else {
        format!("build {err}")
    }
}

/// Start a new log, keeping the previous one as `<log>.1`.
fn open_log(path: &Path) -> Result<File, String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut old = path.as_os_str().to_owned();
    old.push(".1");
    let _ = fs::rename(path, old);
    File::create(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The last 4 KiB of the log.
fn tail(path: &Path) -> String {
    let Ok(mut file) = File::open(path) else { return String::new() };
    let len = file.metadata().map_or(0, |m| m.len());
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(4096)));
    let mut bytes = vec![];
    let _ = file.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The log's last non-empty line, at most `LINE_MAX` characters.
fn last_line(path: &Path) -> String {
    let tail = tail(path);
    let line = tail.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    line.chars().take(LINE_MAX).collect()
}

#[cfg(test)]
mod tests;
