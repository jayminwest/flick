use std::fs;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::*;
use crate::core::{Event, ItemId, Module, test_cx};
use crate::modules::apps::App;

/// How long a test waits for a sizing thread; generous for a loaded CI machine (mx-22c2a1).
const WAIT: Duration = Duration::from_secs(30);

/// A temp home with `Applications/FlickTest.app` (bundle id `bid`), two leftovers and a decoy,
/// removed on drop. Its "Trash" is a folder inside it: tests never use the real Trash.
struct Home {
    dir: PathBuf,
    app: PathBuf,
    bid: String,
}

impl Home {
    fn new(test: &str) -> Home {
        let dir = std::env::temp_dir().join(format!("flk-{}-uninst-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Library")).unwrap();
        fs::create_dir_all(dir.join("Trash")).unwrap();
        let dir = dir.canonicalize().unwrap();
        // Unique per test: NSBundle caches bundles by path for the life of the process.
        let bid = format!("dev.flick.uninstall-test-{test}");
        let app = dir.join("Applications/FlickTest.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>CFBundleIdentifier</key><string>{bid}</string>\
             <key>CFBundleExecutable</key><string>FlickTest</string></dict></plist>"
        );
        fs::write(app.join("Contents/Info.plist"), plist).unwrap();
        fs::write(app.join("Contents/MacOS/FlickTest"), vec![b'x'; 100]).unwrap();
        let home = Home { dir, app, bid };
        home.file(&format!("Caches/{}/a", home.bid), 2000);
        home.file(&format!("Preferences/{}.plist", home.bid), 7);
        home.file(&format!("Caches/{}-other/a", home.bid), 1);
        home
    }

    fn file(&self, rel: &str, bytes: usize) -> PathBuf {
        let path = self.lib(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'x'; bytes]).unwrap();
        path
    }

    fn lib(&self, rel: &str) -> PathBuf {
        self.dir.join("Library").join(rel)
    }

    fn caches(&self) -> PathBuf {
        self.lib(&format!("Caches/{}", self.bid))
    }

    fn prefs(&self) -> PathBuf {
        self.lib(&format!("Preferences/{}.plist", self.bid))
    }

    /// Bytes of the app bundle (its Info.plist length varies with the bundle id).
    fn bundle_bytes(&self) -> u64 {
        leftovers::size(&self.app).bytes
    }

    fn apps(&self, own: Option<PathBuf>) -> Apps {
        let app = App { name: "FlickTest".into(), path: self.app.clone() };
        Apps { home: self.dir.clone(), own, ..Apps::new(vec![app]) }
    }

    /// A stand-in for `files::trash`: moves into this home's Trash folder and logs the path.
    fn trash<'a>(
        &'a self,
        log: &'a mut Vec<PathBuf>,
    ) -> impl FnMut(&Path) -> Result<PathBuf, String> + 'a {
        move |p: &Path| {
            let to = self.dir.join("Trash").join(log.len().to_string());
            fs::rename(p, &to).map_err(|e| e.to_string())?;
            log.push(p.to_path_buf());
            Ok(to)
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_string()).collect()
}

fn status(outcome: Outcome) -> String {
    match outcome {
        Outcome::Stay(Some(s)) => s,
        _ => panic!("not a status"),
    }
}

fn confirm(outcome: Outcome) -> Confirm {
    match outcome {
        Outcome::Confirm(c) => c,
        Outcome::Stay(s) => panic!("no confirmation: {s:?}"),
        _ => panic!("no confirmation"),
    }
}

fn bytes(n: u64) -> String {
    human(Size { bytes: n, capped: false })
}

/// The sizing thread's notice; tests wait on the sizes instead.
fn quiet() {}

/// Wait until the pending plan's sizing thread is done.
fn wait_sized(apps: &Apps) {
    let start = Instant::now();
    let sized = |a: &Apps| a.pending.as_ref().is_some_and(|p| p.total(|_| true).is_some());
    while !sized(apps) {
        assert!(start.elapsed() < WAIT, "sizing never finished");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Uninstall… then Enter in the review view once the sizes are in: the confirmation.
fn ask(apps: &mut Apps, path: &Path) -> Confirm {
    assert!(matches!(apps.ask_uninstall(path, quiet), Outcome::Push(v) if v.is("app", VIEW)));
    wait_sized(apps);
    confirm(apps.confirm_pending())
}

/// The answer a listing verb sends later, from its sizing thread.
fn later_answer(rx: Option<Receiver<later::Answer>>) -> later::Answer {
    rx.expect("a later answer").recv_timeout(WAIT).expect("the sizing thread answers")
}

#[test]
fn uninstall_reviews_the_sized_paths_then_confirms_them() {
    let h = Home::new("confirm");
    let mut apps = h.apps(None);
    let id = ItemId::new("app", h.app.display());
    let menu = test_cx("", |cx| apps.actions(&id, cx));
    assert_eq!(menu.last().map(|a| a.title.as_str()), Some("Uninstall…"));

    // Uninstall… pushes the review view; the sizes arrive from the sizing thread.
    let pushed = test_cx("", |cx| apps.act(&id, "uninstall", cx));
    let Outcome::Push(request) = pushed else { panic!("no review view") };
    let mut view = test_cx("", |cx| apps.open(&request.name, cx)).unwrap();
    wait_sized(&apps);
    test_cx("", |cx| apps.refresh(&mut view, cx));
    let total = bytes(h.bundle_bytes() + 2007);
    assert_eq!(view.footer, format!("Enter to move 3 items ({total}) to Trash…"));
    let caches = format!("~/Library/Caches/{}", h.bid);
    let prefs = format!("~/Library/Preferences/{}.plist", h.bid);
    let app_size = bytes(h.bundle_bytes());
    let want = [
        ("~/Applications/FlickTest.app", "Application", app_size.as_str()),
        (caches.as_str(), "", "2.0 KB"),
        (prefs.as_str(), "", "7 bytes"),
    ];
    let rows: Vec<(&str, &str, &str)> = view
        .items
        .iter()
        .map(|r| (r.title.as_str(), r.subtitle.as_str(), r.accessory.as_str()))
        .collect();
    assert_eq!(rows, want);
    // Review rows have no actions; Enter on any of them asks to confirm.
    let row = view.items[1].id.clone();
    assert!(test_cx("", |cx| apps.actions(&row, cx)).is_empty());
    let c = confirm(test_cx("", |cx| apps.activate(&row, cx)));
    assert_eq!((c.module, c.title.as_str()), ("app", "Uninstall FlickTest"));
    assert!(c.destructive);
    assert_eq!(c.label, format!("Move 3 items ({total}) to Trash"));
    let rows: Vec<(&str, &str, &str)> = c
        .rows
        .iter()
        .map(|r| (r.title.as_str(), r.subtitle.as_str(), r.accessory.as_str()))
        .collect();
    assert_eq!(rows, want);
    // Asking moved nothing.
    assert!(h.app.exists() && h.caches().exists());
}

#[test]
fn confirming_moves_exactly_the_listed_paths() {
    let h = Home::new("move");
    let mut apps = h.apps(None);
    let total = bytes(h.bundle_bytes() + 2007);
    let c = ask(&mut apps, &h.app);
    let mut view = view();
    let row = ItemId::new("app", h.app.display()).with_arg(ARG);
    let mut log = vec![];
    // A token that is not the pending one moves nothing.
    let stale = test_cx("", |cx| apps.confirmed("uninstall/elsewhere", cx));
    assert_eq!(status(stale), NOTHING);
    // `confirmed` itself trashes with `TRASH`, which refuses in tests.
    let refused = test_cx("", |cx| apps.confirmed(&c.token, cx));
    let want = "Could not move FlickTest to the Trash: tests never use the Trash; nothing was moved";
    assert_eq!(status(refused), want);
    assert!(h.app.exists() && h.caches().exists());
    // The plan is spent: the review view empties and Enter there says so.
    test_cx("", |cx| apps.refresh(&mut view, cx));
    assert!(view.items.is_empty() && view.footer.is_empty());
    assert_eq!(status(test_cx("", |cx| apps.activate(&row, cx))), NOTHING);

    let c = ask(&mut apps, &h.app);
    // A leftover created after the confirmation is not trashed: only listed paths move.
    let late = h.file(&format!("Logs/{}/x", h.bid), 1);
    let done = apps.confirm_uninstall(&c.token, h.trash(&mut log));
    assert_eq!(status(done), format!("Moved 3 items ({total}) to Trash"));
    assert_eq!(log, [h.app.clone(), h.caches(), h.prefs()]);
    assert!(late.exists() && h.lib(&format!("Caches/{}-other/a", h.bid)).exists());
    assert!(apps.apps.is_empty(), "the app leaves the index");
    // The token is spent.
    let again = apps.confirm_uninstall(&c.token, h.trash(&mut log));
    assert_eq!(status(again), NOTHING);
}

#[test]
fn sizes_are_unknown_until_the_sizing_thread_reports() {
    let h = Home::new("sizing");
    let mut apps = h.apps(None);
    // A plan that was never sized: every size reads "Sizing…" and totals count items only.
    let plan = apps.plan(&h.app).unwrap();
    let c = plan.confirm(&h.dir);
    assert_eq!(c.label, "Move 3 items to Trash");
    assert!(c.rows.iter().all(|r| r.accessory == SIZING));
    let want = [
        format!("{SIZING}\t{}", h.app.display()),
        format!("{SIZING}\t{}", h.caches().display()),
        format!("{SIZING}\t{}", h.prefs().display()),
        format!("{SIZING}\ttotal"),
    ];
    assert_eq!(plan.listing(), want.join("\n"));
    apps.pending = Some(plan.clone());
    let mut view = view();
    apps.refresh_uninstall(&mut view);
    assert_eq!(view.footer, "Enter to move 3 items to Trash…");
    assert!(view.items.iter().all(|i| i.accessory == SIZING && i.id.arg() == Some(ARG)));
    // Moving before the sizes are in still works; the status counts items only.
    let mut log = vec![];
    let done = apps.confirm_uninstall(&plan.token(), h.trash(&mut log));
    assert_eq!(status(done), "Moved 3 items to Trash");

    // The sizing thread fills a clone's sizes too, then calls its `done`.
    let h = Home::new("sizing2");
    let plan = h.apps(None).plan(&h.app).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    plan.clone().start_sizing(move || tx.send(()).unwrap());
    rx.recv_timeout(WAIT).unwrap();
    assert_eq!(plan.total(|_| true).map(|t| t.bytes), Some(h.bundle_bytes() + 2007));
}

#[test]
fn the_sizing_notice_makes_the_review_view_stale() {
    let mut apps = Apps::new(vec![]);
    test_cx("", |cx| {
        assert!(apps.on_event(Event::ModuleChanged { module: "app" }, cx));
        assert!(!apps.on_event(Event::ModuleChanged { module: "kota" }, cx));
        // With nothing pending the review view is empty.
        let mut view = apps.open(VIEW, cx).unwrap();
        assert_eq!(view.empty, "Nothing to uninstall");
        apps.refresh(&mut view, cx);
        assert!(view.items.is_empty());
    });
    assert_eq!(status(apps.confirm_pending()), NOTHING);
}

#[test]
fn changed_leftovers_are_skipped_and_failures_do_not_stop_the_rest() {
    let h = Home::new("changed");
    h.file(&format!("Logs/{}/x", h.bid), 1);
    let mut apps = h.apps(None);
    let c = ask(&mut apps, &h.app);
    // The Caches folder becomes a symlink; Logs fails to move; Preferences still moves.
    let caches = h.caches();
    fs::rename(&caches, h.dir.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(h.dir.join("elsewhere"), &caches).unwrap();
    let logs = h.lib(&format!("Logs/{}", h.bid));
    let mut log = vec![];
    let moved = bytes(h.bundle_bytes() + 7);
    let done = {
        let mut inner = h.trash(&mut log);
        let trash = |p: &Path| if p == logs { Err("denied".into()) } else { inner(p) };
        status(apps.confirm_uninstall(&c.token, trash))
    };
    let want = format!(
        "Moved 2 items ({moved}) to Trash; skipped 1 that changed; 1 could not move ({}: denied)",
        logs.display()
    );
    assert_eq!(done, want);
    assert!(fs::symlink_metadata(&caches).is_ok() && logs.exists());
    assert_eq!(log, [h.app.clone(), h.prefs()]);
}

#[test]
fn a_bundle_that_cannot_move_or_changed_leaves_everything_in_place() {
    let h = Home::new("stuck");
    let mut apps = h.apps(None);
    let c = ask(&mut apps, &h.app);
    let done = status(apps.confirm_uninstall(&c.token, |_: &Path| Err("locked".into())));
    assert_eq!(done, "Could not move FlickTest to the Trash: locked; nothing was moved");
    assert!(h.caches().exists());
    assert_eq!(apps.apps.len(), 1);

    // Now a different app between asking and confirming: nothing moves.
    let mut log = vec![];
    let c = ask(&mut apps, &h.app);
    if let Some(p) = apps.pending.as_mut() {
        p.bid = "dev.flick.other".into();
    }
    let done = status(apps.confirm_uninstall(&c.token, h.trash(&mut log)));
    assert_eq!(done, "FlickTest changed; nothing was moved");

    // Gone between asking and confirming: nothing moves.
    let c = ask(&mut apps, &h.app);
    fs::rename(&h.app, h.dir.join("moved.app")).unwrap();
    let done = status(apps.confirm_uninstall(&c.token, h.trash(&mut log)));
    assert_eq!(done, format!("FlickTest is no longer at {}", h.app.display()));
    assert!(log.is_empty() && h.caches().exists());
}

#[test]
fn protected_apps_are_refused_with_a_reason() {
    let h = Home::new("protected");
    let mut apps = h.apps(Some(h.app.clone()));
    assert!(!apps.uninstallable(&h.app));
    let id = ItemId::new("app", h.app.display());
    let menu = test_cx("", |cx| apps.actions(&id, cx));
    assert!(menu.iter().all(|a| a.title != "Uninstall…"));
    let refusal = status(test_cx("", |cx| apps.act(&id, "uninstall", cx)));
    assert_eq!(refusal, "Cannot uninstall FlickTest: it is Flick");
    let safari = Path::new("/System/Applications/Safari.app");
    assert!(!apps.uninstallable(safari));
    let refusal = status(apps.ask_uninstall(safari, quiet));
    assert_eq!(refusal, "Cannot uninstall Safari: it is part of macOS");
    assert!(apps.pending.is_none());
    let never = |_: &Path| -> Result<PathBuf, String> { Err("never called".into()) };
    let err = apps.uninstall_command(&args(&["flicktest", "--dry-run"]), never);
    assert_eq!(err.unwrap_err(), "app: Cannot uninstall FlickTest: it is Flick");
    assert!(later::take().is_none(), "a refusal answers at once");

    // No home folder: refused rather than searching a relative `Library`.
    let mut homeless = h.apps(None);
    homeless.home = PathBuf::new();
    let refusal = status(homeless.ask_uninstall(&h.app, quiet));
    assert_eq!(refusal, "Cannot uninstall FlickTest: no home folder");
    assert!(h.app.exists() && h.caches().exists());
}

#[test]
fn cli_lists_on_dry_run_refuses_without_yes_and_moves_with_it() {
    let h = Home::new("cli");
    let mut apps = h.apps(None);
    let mut log = vec![];
    let listing = [
        format!("{}\t{}", bytes(h.bundle_bytes()), h.app.display()),
        format!("2.0 KB\t{}", h.caches().display()),
        format!("7 bytes\t{}", h.prefs().display()),
        format!("{}\ttotal", bytes(h.bundle_bytes() + 2007)),
    ]
    .join("\n");
    // `--dry-run` answers at once without sizes, then later with them.
    let dry = apps.uninstall_command(&args(&["flicktest", "--dry-run"]), h.trash(&mut log));
    assert!(dry.unwrap().ends_with(&format!("{SIZING}\ttotal")));
    assert_eq!(later_answer(later::take()), Ok(listing.clone()));
    // No flag: the sized listing comes later, as an error.
    let bare = apps.uninstall_command(&args(&["FlickTest"]), h.trash(&mut log));
    assert!(bare.is_ok(), "the error comes later");
    let bare = later_answer(later::take()).unwrap_err();
    assert_eq!(bare, format!("{listing}\napp: pass --yes to move these to the Trash"));
    let usage = "app: usage: app uninstall <name> [--dry-run | --yes]";
    for bad in [&["--yes"][..], &["FlickTest", "--yes", "--dry-run"], &["FlickTest", "--force"]] {
        let err = apps.uninstall_command(&args(bad), h.trash(&mut log)).unwrap_err();
        assert_eq!(err, usage);
    }
    let err = apps.uninstall_command(&args(&["Nope", "--yes"]), h.trash(&mut log));
    assert_eq!(err.unwrap_err(), "app: no app named \"Nope\"");
    assert!(later::take().is_none());
    assert!(log.is_empty() && h.app.exists());

    // `--yes` moves at once, without sizing.
    let done = apps.uninstall_command(&args(&["--yes", "FlickTest"]), h.trash(&mut log)).unwrap();
    assert!(later::take().is_none());
    let want = [
        format!("moved\t{}", h.app.display()),
        format!("moved\t{}", h.caches().display()),
        format!("moved\t{}", h.prefs().display()),
        "Moved 3 items to Trash".to_string(),
    ];
    assert_eq!(done, want.join("\n"));
    let err = apps.uninstall_command(&args(&["FlickTest", "--yes"]), h.trash(&mut log));
    assert_eq!(err.unwrap_err(), "app: no app named \"FlickTest\"");
}

#[test]
fn the_command_verb_reaches_uninstall() {
    let h = Home::new("verb");
    let mut apps = h.apps(None);
    let out = test_cx("", |cx| apps.command(&args(&["uninstall", "FlickTest", "--dry-run"]), cx));
    assert!(out.unwrap().ends_with("\ttotal"));
    assert!(later_answer(later::take()).unwrap().ends_with("\ttotal"));
    let out = test_cx("", |cx| apps.command(&args(&["uninstall", "FlickTest", "--yes"]), cx));
    let want = "app: Could not move FlickTest to the Trash: tests never use the Trash; nothing was moved";
    assert_eq!(out.unwrap_err(), want);
    assert!(h.app.exists());
}

#[test]
fn report_lines_list_every_path() {
    let sizes = Some(vec![Size::default(), Size::default()]);
    let plan = Plan {
        name: "X".into(),
        bid: "dev.x".into(),
        paths: vec!["/a".into(), "/b".into()],
        sizes: Arc::new(Mutex::new(sizes)),
    };
    let report = Report {
        moved: vec!["/a".into()],
        skipped: vec!["/c".into()],
        failed: vec![("/b".into(), "denied".into())],
    };
    let want = "moved\t/a\nskipped\t/c\nfailed\t/b\tdenied\nMoved 1 item (0 bytes) to Trash; \
                skipped 1 that changed; 1 could not move (/b: denied)";
    assert_eq!(report.lines(&plan), want);
}

#[test]
fn sizes_read_like_finder() {
    let s = |bytes, capped| human(Size { bytes, capped });
    assert_eq!(s(0, false), "0 bytes");
    assert_eq!(s(1, false), "1 byte");
    assert_eq!(s(999, false), "999 bytes");
    assert_eq!(s(1000, false), "1.0 KB");
    assert_eq!(s(1_250_000, true), "≥ 1.2 MB");
    assert_eq!(s(999_960, false), "1.0 MB");
    assert_eq!(s(3_000_000_000, false), "3.0 GB");
    assert_eq!(s(5_000_000_000_000_000, false), "5000.0 TB");
    assert_eq!(shown(None), SIZING);
    assert_eq!(tilde(Path::new("/h/Library/x"), Path::new("/h")), "~/Library/x");
    assert_eq!(tilde(Path::new("/o/x"), Path::new("/h")), "/o/x");
    assert_eq!(tilde(Path::new("/o/x"), Path::new("")), "/o/x");
}
