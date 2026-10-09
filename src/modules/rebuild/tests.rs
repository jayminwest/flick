use super::*;
use crate::config::parse;
use crate::core::test_cx;

const INSTALLED: &str = "1111111111111111111111111111111111111111";
const HEAD: &str = "2222222222222222222222222222222222222222";

fn module(sha: Option<&str>) -> Rebuild {
    let stamp = Stamp::from_parts(sha, Some("0"), Some("2026-10-09T18:01:55Z"), "/src/flick");
    Rebuild::new(stamp)
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
    test_cx("", |cx| m.items(cx)).into_iter().map(|i| (i.id.to_string(), i.subtitle)).collect()
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
        test_cx("", |cx| m.command(&words(&["rebuild"]), cx)).unwrap_err(),
        "flick: unknown command \"rebuild\""
    );
    assert_eq!(m.verbs(), "flick version");

    let ok = m.version_text(&Ok(Status { dirty: true, ..newer(1, &["feat: x"]) }));
    assert!(ok.contains(&format!("checkout  {HEAD} dirty 1 newer\n  2222222 feat: x")), "{ok}");
    let unknown = m.version_text(&Ok(Status { newer: None, ..newer(0, &[]) }));
    assert!(unknown.ends_with(&format!("checkout  {HEAD} clean")), "{unknown}");
    assert!(module(None).version_text(&Err("e".into())).contains("installed dev clean built 2026"));

    let run = |m: &mut Rebuild, key: &str| test_cx("", |cx| m.activate(&ItemId::new("flick", key), cx));
    let Outcome::Stay(Some(v)) = run(&mut m, "version") else { panic!("version shows text") };
    assert!(v.starts_with("Flick ") && v.contains("1111111"), "{v}");
    assert!(matches!(run(&mut m, "rebuild-available"), Outcome::Stay(Some(t)) if t.contains("bundle.sh")));
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
