use super::*;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Failed builds that asked to open their log, across tests.
static OPENED: AtomicUsize = AtomicUsize::new(0);

const QUIET: Hooks = Hooks { notify: || {}, open_log: |_| {} };
const COUNTING: Hooks = Hooks {
    notify: || {},
    open_log: |_| {
        OPENED.fetch_add(1, Ordering::SeqCst);
    },
};

/// A stand-in bundle.sh: reports its env, honours `fail` and `slow` marker files, and
/// makes an empty bundle where the real one would.
const BUNDLE: &str = r#"#!/bin/sh
echo "args $FLICK_CARGO_ARGS sha=$FLICK_BUILD_SHA dirty=$FLICK_BUILD_DIRTY source=$FLICK_BUILD_SOURCE time=${FLICK_BUILD_TIME:-unset}"
[ -e uncommitted.txt ] && echo "saw uncommitted"
[ -e fail ] && { echo "error: boom"; exit 1; }
[ -e slow ] && sleep 30
mkdir -p "$CARGO_TARGET_DIR/Flick.app"
echo built
"#;

/// A stand-in relaunch.sh that only records its arguments.
const RELAUNCH: &str = "#!/bin/sh\necho \"relaunch $*\"\necho \"$*\" > \"$FLICK_INSTALL_DIR/relaunched\"\n";

/// A temp dir holding a git checkout (`src`) with the stand-in scripts committed, an
/// install dir and the runner's paths.
struct Fixture {
    dir: PathBuf,
    runner: Runner,
}

impl Fixture {
    fn new(name: &str, hooks: Hooks) -> Fixture {
        let dir = std::env::temp_dir().join(format!("flk-{}-build-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let src = dir.join("src");
        fs::create_dir_all(src.join("scripts")).unwrap();
        fs::create_dir_all(dir.join("install")).unwrap();
        for (file, body) in [("bundle.sh", BUNDLE), ("relaunch.sh", RELAUNCH)] {
            let path = src.join("scripts").join(file);
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let sh = |args: &[&str]| {
            let ok = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .arg("-C")
                .arg(&src)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        sh(&["init", "-q"]);
        sh(&["add", "."]);
        sh(&["commit", "-q", "-m", "scripts"]);
        let paths = Paths { log: dir.join("logs/rebuild.log"), export: dir.join("export/src") };
        Fixture { runner: Runner::new(paths, hooks), dir }
    }

    fn src(&self) -> PathBuf {
        self.dir.join("src")
    }

    fn request(&self, rev: Option<&str>) -> Request {
        Request {
            source: self.src(),
            rev: rev.map(String::from),
            gates: false,
            restart: false,
            install_dir: Some(self.dir.join("install")),
            open_log: true,
        }
    }

    /// Start `req` and wait for it to end.
    fn build(&self, req: Request) -> Progress {
        self.runner.start(req).unwrap();
        self.wait()
    }

    fn wait(&self) -> Progress {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.runner.progress().active() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        self.runner.progress()
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.runner.paths.log).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_head_build_leaves_uncommitted_edits_out_and_installs() {
    let f = Fixture::new("head", QUIET);
    fs::write(f.src().join("uncommitted.txt"), "x").unwrap();
    let p = f.build(f.request(Some("HEAD")));
    assert_eq!(p.phase, Phase::Installed, "{}", f.log());
    assert_eq!(p.label.len(), 7);
    let log = f.log();
    assert!(log.contains("args --locked --offline sha="), "{log}");
    assert!(log.contains(&format!("dirty=0 source={} time=unset", f.src().display())), "{log}");
    assert!(!log.contains("saw uncommitted"), "{log}");
    assert!(f.runner.paths.export.join("scripts/bundle.sh").exists());
    let app = f.src().join("target/flick-rebuild/Flick.app");
    let relaunched = fs::read_to_string(f.dir.join("install/relaunched")).unwrap();
    assert_eq!(relaunched.trim(), format!("--install {} --no-restart", app.display()));
    assert_eq!(p.last_line, format!("relaunch --install {} --no-restart", app.display()));
    assert!(p.summary(Instant::now()).starts_with(&format!("installed {} after ", p.label)));

    // The next build keeps this log as rebuild.log.1.
    let again = f.build(f.request(Some("HEAD~0")));
    assert_eq!(again.phase, Phase::Installed);
    let mut old = f.runner.paths.log.clone().into_os_string();
    old.push(".1");
    assert_eq!(fs::read_to_string(old).unwrap(), log);
}

#[test]
fn a_dirty_build_takes_the_tree_and_a_failure_installs_nothing() {
    let f = Fixture::new("dirty", COUNTING);
    fs::write(f.src().join("uncommitted.txt"), "x").unwrap();
    let p = f.build(f.request(None));
    assert_eq!(p.phase, Phase::Installed, "{}", f.log());
    assert!(p.label.ends_with("-dirty") && p.label.len() == 13, "{}", p.label);
    assert!(f.log().contains("dirty=1") && f.log().contains("saw uncommitted"));

    fs::remove_file(f.dir.join("install/relaunched")).unwrap();
    fs::write(f.src().join("fail"), "").unwrap();
    let opened = OPENED.load(Ordering::SeqCst);
    let p = f.build(f.request(None));
    assert_eq!(p.phase, Phase::Failed("build failed (exit status: 1)".into()), "{}", f.log());
    assert_eq!(p.title(Instant::now()), "Build Failed (see log)");
    assert!(f.log().contains("error: boom"));
    assert!(!f.dir.join("install/relaunched").exists(), "a failed build installs nothing");
    assert!(OPENED.load(Ordering::SeqCst) > opened, "the log opens");
}

#[test]
fn one_build_at_a_time_and_cancel_stops_it() {
    let f = Fixture::new("cancel", QUIET);
    assert_eq!(f.runner.cancel(), "no build running");
    fs::write(f.src().join("slow"), "").unwrap();
    let reply = f.runner.start(f.request(None)).unwrap();
    assert!(reply.starts_with("building the dirty tree of "), "{reply}");
    assert!(reply.ends_with("logs/rebuild.log"), "{reply}");
    assert!(f.runner.start(f.request(None)).unwrap_err().starts_with("already building "));
    // Wait for the stand-in bundle.sh to be running (and sleeping) before cancelling.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !f.runner.progress().last_line.contains("args") && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let started = Instant::now();
    assert!(f.runner.cancel().starts_with("cancelling build "));
    let p = f.wait();
    assert_eq!(p.phase, Phase::Cancelled, "{}", f.log());
    assert!(started.elapsed() < Duration::from_secs(20), "sleep 30 was killed");
    assert!(!f.dir.join("install/relaunched").exists());
    assert!(p.summary(Instant::now()).starts_with("cancelled "));
}

#[test]
fn unknown_and_option_like_revs_fail_before_building() {
    let f = Fixture::new("revs", QUIET);
    let p = f.build(f.request(Some("no-such-branch")));
    assert_eq!(p.phase, Phase::Failed(format!("git rev-parse failed in {}", f.src().display())));
    let p = f.build(f.request(Some("--output=/tmp/x")));
    assert_eq!(p.phase, Phase::Failed("bad rev \"--output=/tmp/x\"".into()));
    assert!(f.log().ends_with("flick rebuild: bad rev \"--output=/tmp/x\"\n"));
}

#[test]
fn progress_text_for_each_phase() {
    let now = Instant::now();
    let started = now.checked_sub(Duration::from_secs(42));
    let at = |phase| Progress { phase, label: "abc1234".into(), started, ..Progress::default() };
    let idle = Progress::default();
    assert_eq!((idle.summary(now), idle.title(now), idle.secs(now)), ("idle".into(), "No Build Running".into(), 0));
    assert_eq!(at(Phase::Building).summary(now), "building 42s abc1234");
    assert_eq!(at(Phase::Building).title(now), "Building abc1234… 42s");
    assert_eq!(at(Phase::Installing).summary(now), "installing abc1234");
    assert_eq!(at(Phase::Installing).title(now), "Installing abc1234…");
    assert!(at(Phase::Installing).active() && !at(Phase::Installed).active());
    assert_eq!(at(Phase::Installed).title(now), "Installed abc1234");
    assert_eq!(at(Phase::Failed("x".into())).summary(now), "failed abc1234: x");
    assert_eq!(at(Phase::Cancelled).title(now), "Build Cancelled");
    let ended = Progress { ended: started.map(|s| s + Duration::from_secs(5)), ..at(Phase::Cancelled) };
    assert_eq!(ended.summary(now), "cancelled abc1234 after 5s");
}

#[test]
fn failures_explain_known_causes() {
    const NO_CARGO: &str = "cargo not found on login-shell PATH: install Rust with rustup";
    let src = Path::new("/src/flick");
    assert_eq!(explain("failed (x)", "zsh:1: command not found: cargo\n", src), NO_CARGO);
    assert_eq!(explain("failed (x)", "bundle.sh: cargo not found: install Rust", src), NO_CARGO);
    assert_eq!(
        explain("failed (x)", "error: no matching package named `foo` found\nAs a reminder, you're using offline mode (--offline)", src),
        "build failed (x); a crate may be missing offline: run `cargo fetch` in /src/flick"
    );
    assert_eq!(explain("failed (x)", "error[E0425]", src), "build failed (x)");
    assert_eq!(explain("failed (x)", "cargo build --locked --offline", src), "build failed (x)");
}

#[test]
fn last_line_reads_the_tail_and_caps_its_length() {
    let dir = std::env::temp_dir().join(format!("flk-{}-build-tail", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let log = dir.join("a/b.log");
    assert_eq!(last_line(&log), "");
    let mut file = open_log(&log).unwrap();
    write!(file, "{}\nfirst\n  second  \n\n", "x".repeat(5000)).unwrap();
    assert_eq!(last_line(&log), "second");
    writeln!(file, "{}", "y".repeat(500)).unwrap();
    assert_eq!(last_line(&log), "y".repeat(LINE_MAX));
    assert!(Paths::standard().log.ends_with("Library/Logs/Flick/rebuild.log"));
    let _ = fs::remove_dir_all(&dir);
}

/// Run this checkout's real scripts/bundle.sh --install under /bin/bash (3.2 on macOS) with
/// `cargo` (a shell script, or `None` for no cargo) as the only cargo on PATH. A stale
/// Flick.app sits in the target dir. Returns (exit code, stderr, target dir, install dir).
fn real_bundle(name: &str, cargo: Option<&str>) -> (Option<i32>, String, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("flk-{}-bundle-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let (bin, target, install) = (dir.join("bin"), dir.join("target"), dir.join("install"));
    fs::create_dir_all(target.join("Flick.app")).unwrap();
    fs::create_dir_all(&bin).unwrap();
    if let Some(body) = cargo {
        fs::write(bin.join("cargo"), format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(bin.join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = Command::new("/bin/bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/bundle.sh"))
        .args(["--install", "--no-restart"])
        .env("PATH", format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()))
        .env("CARGO_TARGET_DIR", &target)
        .env("FLICK_INSTALL_DIR", &install)
        .env_remove("FLICK_CARGO_ARGS")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned(), target, install)
}

#[test]
fn bundle_sh_fails_without_cargo_and_installs_nothing() {
    let cases = [
        ("missing", None, "cargo not found: install Rust with rustup"),
        ("no-toolchain", Some("echo 'error: no default toolchain' >&2; exit 1"), "cargo does not run"),
        ("fails", Some("[ \"$1\" = --version ] && exit 0; echo 'error: boom' >&2; exit 101"), "cargo build failed"),
        ("no-binary", Some("exit 0"), "cargo made no binary"),
    ];
    for (name, cargo, message) in cases {
        let (code, stderr, target, install) = real_bundle(name, cargo);
        assert_eq!(code, Some(1), "{name}: {stderr}");
        assert!(stderr.contains(message) && stderr.contains("nothing installed"), "{name}: {stderr}");
        assert!(!install.exists(), "{name}: relaunch.sh ran");
        assert!(!target.join("Flick.app").exists(), "{name}: stale bundle left to install");
        let _ = fs::remove_dir_all(target.parent().unwrap());
    }
}

/// The real thing: export this checkout's HEAD, `cargo build --release --locked --offline`,
/// bundle, sign, and install into a temp dir without restarting anything. Minutes long.
#[test]
#[ignore = "builds Flick in release mode; run with --ignored"]
fn end_to_end_build_into_a_temp_install_dir() {
    let dir = std::env::temp_dir().join(format!("flk-{}-build-e2e", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let paths = Paths { log: dir.join("rebuild.log"), export: dir.join("export/src") };
    let runner = Runner::new(paths, QUIET);
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let install = dir.join("install");
    let req = Request {
        source,
        rev: Some("HEAD".into()),
        gates: false,
        restart: false,
        install_dir: Some(install.clone()),
        open_log: false,
    };
    runner.start(req).unwrap();
    let deadline = Instant::now() + Duration::from_mins(30);
    while runner.progress().active() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(500));
    }
    let log = fs::read_to_string(&runner.paths.log).unwrap();
    assert_eq!(runner.progress().phase, Phase::Installed, "{log}");
    assert!(log.contains("--offline"), "{log}");
    assert!(install.join("Flick.app/Contents/MacOS/Flick").exists());
    let _ = fs::remove_dir_all(&dir);
}
