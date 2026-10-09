use super::*;
use crate::config::parse;
use crate::core::test_cx;

const INSTALLED: &str = "1111111111111111111111111111111111111111";
const HEAD: &str = "2222222222222222222222222222222222222222";

/// A module over checkout /src/flick whose builds never restart Flick, post events or
/// open editors, and log to a temp dir.
fn module(sha: Option<&str>) -> Rebuild {
    // Each module gets its own log dir: tests run in parallel and rotate logs.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let stamp = Stamp::from_parts(sha, Some("0"), Some("2026-10-09T18:01:55Z"), "/src/flick");
    let mut m = Rebuild::new(stamp);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("flk-{}-rebuild-mod{n}", std::process::id()));
    let paths = Paths { log: dir.join("logs/rebuild.log"), export: dir.join("export") };
    m.runner = Runner::new(paths, Hooks { notify: || {}, open_log: |_| {} });
    m.restart = false;
    m.install_dir = Some(dir.join("install"));
    m
}

/// Wait for a build to end.
fn settle(m: &Rebuild) -> build::Progress {
    let deadline = Instant::now() + Duration::from_secs(30);
    while m.runner.progress().active() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    m.runner.progress()
}

fn configure(m: &mut Rebuild, text: &str) -> Result<(), String> {
    m.configure(&parse(text)?.section("flick")?.unwrap())
}

fn with_status(m: &Rebuild, status: Result<Status, String>) {
    Rebuild::store(&m.cache, m.source.clone(), status);
}

fn newer(n: usize, subjects: &[&str]) -> Status {
    Status {
        head: HEAD.into(),
        dirty: false,
        newer: Some(n),
        subjects: subjects.iter().map(|s| ("2222222".to_string(), (*s).to_string())).collect(),
    }
}

fn items(m: &mut Rebuild) -> Vec<(String, String)> {
    test_cx("", |cx| m.items(cx))
        .into_iter()
        .filter(|i| !i.id.key().starts_with("rebuild") || i.id.key() == "rebuild-available")
        .map(|i| (i.id.to_string(), i.subtitle))
        .collect()
}

#[test]
fn version_shows_the_stamp_before_any_check() {
    let mut m = module(Some(INSTALLED));
    assert_eq!(items(&mut m), [("flick:version".into(), "1111111 built 2026-10-09 18:01 UTC".into())]);
    assert_eq!(items(&mut module(None)), [("flick:version".into(), "dev build".into())]);
}

#[test]
fn rebuild_available_only_when_head_differs() {
    let mut m = module(Some(INSTALLED));
    with_status(&m, Ok(newer(2, &["feat: b", "feat: a"])));
    let shown = items(&mut m);
    assert_eq!(
        shown[0].1,
        "1111111 built 2026-10-09 18:01 UTC · 2 commits newer in /src/flick: feat: b; feat: a"
    );
    assert_eq!(shown[1], ("flick:rebuild-available".into(), "Build 2222222 from /src/flick".into()));

    with_status(&m, Ok(newer(1, &["fix"])));
    assert!(items(&mut m)[0].1.ends_with("1 commit newer in /src/flick: fix"));

    let mut current = module(Some(HEAD));
    with_status(&current, Ok(Status { dirty: true, ..newer(0, &[]) }));
    let shown = items(&mut current);
    assert_eq!(shown.len(), 1);
    assert!(shown[0].1.ends_with("up to date with /src/flick (uncommitted changes)"), "{}", shown[0].1);
}

#[test]
fn unknown_installed_sha_and_errors_still_describe_the_checkout() {
    let mut dev = module(None);
    with_status(&dev, Ok(Status { newer: None, ..newer(0, &[]) }));
    let shown = items(&mut dev);
    assert_eq!(shown[0].1, "dev build · /src/flick is at 2222222");
    assert_eq!(shown.len(), 2, "a dev build can rebuild too");

    let mut broken = module(Some(INSTALLED));
    with_status(&broken, Err("git status failed".into()));
    assert_eq!(items(&mut broken).len(), 1);
    assert!(items(&mut broken)[0].1.ends_with("checkout /src/flick: git status failed"));
}

#[test]
fn a_new_source_ignores_the_old_checkouts_status() {
    let mut m = module(Some(INSTALLED));
    with_status(&m, Ok(newer(1, &["x"])));
    m.last_check = Some(Instant::now());
    configure(&mut m, "[flick]\nsource = \"/elsewhere\"").unwrap();
    assert_eq!(m.source, PathBuf::from("/elsewhere"));
    assert_eq!(m.last_check, None);
    assert_eq!(items(&mut m).len(), 1);
    configure(&mut m, "").unwrap();
    assert_eq!(m.source, PathBuf::from("/src/flick"));
}

#[test]
fn config_rejects_unknown_keys() {
    let mut m = module(None);
    let err = configure(&mut m, "[flick]\nsorce = \"/x\"").unwrap_err();
    assert!(err.starts_with("[flick]: unknown field `sorce`"), "{err}");
    configure(&mut m, "[flick]\ncheck_on_open = false").unwrap();
    assert!(!m.check_on_open);
}

#[test]
fn checks_run_on_open_and_wake_at_most_every_30s() {
    let mut m = module(Some(INSTALLED));
    m.source = std::env::temp_dir().join(format!("flk-{}-rebuild-none", std::process::id()));
    test_cx("", |cx| {
        assert!(!m.on_event(Event::PasteboardChanged, cx));
        assert_eq!(m.last_check, None);
        assert!(!m.on_event(Event::LauncherOpened, cx));
        let first = m.last_check.unwrap();
        m.on_event(Event::Wake, cx);
        assert_eq!(m.last_check, Some(first), "throttled");
        m.last_check = first.checked_sub(CHECK_EVERY);
        m.on_event(Event::Wake, cx);
        assert!(m.last_check.unwrap() > first);
    });
    let mut off = module(None);
    configure(&mut off, "[flick]\ncheck_on_open = false").unwrap();
    test_cx("", |cx| off.on_event(Event::LauncherOpened, cx));
    assert_eq!(off.last_check, None);
}

#[test]
fn background_check_fills_the_cache() {
    let mut m = module(None);
    m.source = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    m.spawn_check();
    let deadline = Instant::now() + Duration::from_secs(20);
    while m.status().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    // A worktree or exported tree may not be a git checkout; either way a result arrives.
    assert!(m.status().is_some());
}

#[test]
fn version_command_and_activation() {
    let mut m = module(Some(INSTALLED));
    m.source = std::env::temp_dir().join(format!("flk-{}-rebuild-cmd", std::process::id()));
    let words = |w: &[&str]| w.iter().map(ToString::to_string).collect::<Vec<_>>();
    let text = test_cx("", |cx| m.command(&words(&["version"]), cx)).unwrap();
    assert!(text.contains(&format!("installed {INSTALLED} clean built 2026-10-09T18:01:55Z")), "{text}");
    assert!(text.contains("checkout  error: git"), "{text}");
    assert!(m.status().unwrap().is_err(), "the command refreshes the cache");
    assert_eq!(
        test_cx("", |cx| m.command(&words(&["rebuld"]), cx)).unwrap_err(),
        "flick: unknown command \"rebuld\""
    );
    assert!(m.verbs().starts_with("flick version | flick rebuild [--dirty]"));

    let ok = m.version_text(&Ok(Status { dirty: true, ..newer(1, &["feat: x"]) }));
    assert!(ok.contains(&format!("checkout  {HEAD} dirty 1 newer\n  2222222 feat: x")), "{ok}");
    let unknown = m.version_text(&Ok(Status { newer: None, ..newer(0, &[]) }));
    assert!(unknown.ends_with(&format!("checkout  {HEAD} clean")), "{unknown}");
    assert!(module(None).version_text(&Err("e".into())).contains("installed dev clean built 2026"));

    let run = |m: &mut Rebuild, key: &str| test_cx("", |cx| m.activate(&ItemId::new("flick", key), cx));
    let Outcome::Stay(Some(v)) = run(&mut m, "version") else { panic!("version shows text") };
    assert!(v.starts_with("Flick ") && v.contains("1111111"), "{v}");
    assert!(matches!(run(&mut m, "nope"), Outcome::Stay(None)));
}

#[test]
fn tilde_paths() {
    let home = dirs::home_dir().unwrap();
    assert_eq!(expand("~"), home);
    assert_eq!(expand("~/Projects/flick"), home.join("Projects/flick"));
    assert_eq!(expand("~other/x"), PathBuf::from("~other/x"));
    assert_eq!(expand("/abs"), PathBuf::from("/abs"));
    assert_eq!(tilde(&home.join("Projects/flick")), "~/Projects/flick");
    assert_eq!(tilde(&home), "~");
    assert_eq!(tilde(Path::new("/src/flick")), "/src/flick");
}

#[test]
fn rebuild_items_start_a_build_and_show_its_view() {
    let mut m = module(Some(INSTALLED));
    m.source = std::env::temp_dir().join(format!("flk-{}-rebuild-act", std::process::id()));
    let all = test_cx("", |cx| m.items(cx));
    let keys: Vec<&str> = all.iter().map(|i| i.id.key()).collect();
    assert_eq!(keys, ["version", "rebuild", "rebuild-dirty"]);
    assert!(all[1].subtitle.starts_with("Build HEAD of "), "{}", all[1].subtitle);
    assert!(all[2].subtitle.contains("with uncommitted changes"), "{}", all[2].subtitle);

    let run = |m: &mut Rebuild, key: &str| test_cx("", |cx| m.activate(&ItemId::new("flick", key), cx));
    for key in ["rebuild", "rebuild-available", "rebuild-dirty"] {
        let Outcome::Push(view) = run(&mut m, key) else { panic!("{key} shows the build") };
        assert!(view.is("flick", "build"));
        // Not a git checkout: the build fails at rev-parse, before anything runs.
        assert!(matches!(settle(&m).phase, build::Phase::Failed(e) if e.starts_with("git rev-parse failed")));
    }
    assert_eq!(settle(&m).label, "the dirty tree");
}

#[test]
fn the_build_view_shows_status_cancel_and_log() {
    let mut m = module(Some(INSTALLED));
    m.source = std::env::temp_dir().join(format!("flk-{}-rebuild-view", std::process::id()));
    let run = |m: &mut Rebuild, key: &str| test_cx("", |cx| m.activate(&ItemId::new("flick", key), cx));
    assert!(matches!(run(&mut m, "rebuild"), Outcome::Push(_)));
    settle(&m);
    let mut view = test_cx("", |cx| m.open("build", cx)).unwrap();
    assert!(test_cx("", |cx| m.open("other", cx)).is_none());
    test_cx("", |cx| m.refresh(&mut view, cx));
    let rows: Vec<(String, String)> =
        view.items.iter().map(|i| (i.id.to_string(), i.title.clone())).collect();
    assert_eq!(rows[0], ("flick:build-status".into(), "Build Failed (see log)".into()));
    assert_eq!(rows[1], ("flick:build-log".into(), "Open Build Log".into()));
    assert_eq!(rows.len(), 2, "no Cancel once it ended");

    let Outcome::Stay(Some(cancel)) = run(&mut m, "cancel-build") else { panic!("cancel says") };
    assert_eq!(cancel, "no build running");
    // Without a log there is nothing to open (and no editor starts in a test).
    std::fs::remove_dir_all(m.runner.paths.log.parent().unwrap()).ok();
    assert!(matches!(run(&mut m, "build-log"), Outcome::Stay(Some(t)) if t == "No build log yet"));
}

#[test]
fn rebuild_status_and_cancel_commands() {
    let mut m = module(Some(INSTALLED));
    m.source = std::env::temp_dir().join(format!("flk-{}-rebuild-cli", std::process::id()));
    let words = |w: &[&str]| w.iter().map(ToString::to_string).collect::<Vec<_>>();
    let mut run = |w: &[&str]| test_cx("", |cx| m.command(&words(w), cx));
    let status = run(&["status"]).unwrap();
    assert!(status.starts_with("idle\nlog: ") && status.ends_with("rebuild.log"), "{status}");
    assert_eq!(run(&["cancel"]).unwrap(), "no build running");
    assert_eq!(run(&["rebuild", "--bogus"]).unwrap_err(), "flick rebuild: unknown option \"--bogus\"");
    let reply = run(&["rebuild", "--ref", "main"]).unwrap();
    assert!(reply.starts_with("building main of "), "{reply}");
    settle(&m);
    let status = test_cx("", |cx| m.command(&words(&["status"]), cx)).unwrap();
    assert!(status.starts_with("failed main: git rev-parse failed"), "{status}");
    assert!(status.contains("\n  flick rebuild: git rev-parse failed"), "{status}");
}

#[test]
fn rebuild_options() {
    let src = Path::new("/src/flick");
    let parse = |w: &[&str]| parse_rebuild(&w.iter().map(ToString::to_string).collect::<Vec<_>>(), src);
    assert_eq!(parse(&[]).unwrap(), (Some("HEAD".into()), src.to_path_buf()));
    assert_eq!(parse(&["--dirty"]).unwrap(), (None, src.to_path_buf()));
    assert_eq!(parse(&["--ref", "v1", "--source", "/wt"]).unwrap(), (Some("v1".into()), "/wt".into()));
    assert_eq!(parse(&["--source", "~"]).unwrap().1, dirs::home_dir().unwrap());
    assert!(parse(&["--ref"]).unwrap_err().contains("--ref needs a rev"));
    assert!(parse(&["--source"]).unwrap_err().contains("--source needs a directory"));
    assert!(parse(&["--dirty", "--ref", "x"]).unwrap_err().contains("drop --ref"));
}

#[test]
fn gates_are_off_unless_configured() {
    let mut m = module(None);
    configure(&mut m, "[flick]\ngates = true").unwrap();
    assert!(m.gates && m.request(None, "/x".into(), false).gates);
    configure(&mut m, "").unwrap();
    let req = m.request(Some("HEAD".into()), "/x".into(), true);
    assert!(!req.gates && !req.restart && req.open_log);
    assert!(Rebuild::default().restart, "the real module restarts Flick after install");
}

#[test]
fn the_example_config_lists_every_key() {
    crate::config::example::assert_documents::<Settings>("flick");
}
