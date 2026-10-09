mod app;
mod apps;
mod config;
mod hotkey;
mod raycast;
mod search;
mod spaces;
mod store;
mod ui;
mod windows;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSRunningApplication};
use objc2_foundation::NSBundle;

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "startup invariants: no UI without the main thread or the database"
)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("snapshot") {
        snapshot(&args[2..]);
        return;
    }
    if args.get(1).map(String::as_str) == Some("import-raycast") {
        let Some(path) = args.get(2) else {
            eprintln!("usage: flick import-raycast <Quicklinks.json>");
            std::process::exit(2);
        };
        match raycast::import_file(path.as_ref()) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("flick: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let mtm = MainThreadMarker::new().expect("must start on the main thread");
    if already_running() {
        eprintln!("flick: already running");
        return;
    }
    let ns_app = NSApplication::sharedApplication(mtm);
    // No Dock icon or menu bar.
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let config = config::load().unwrap_or_else(|e| {
        eprintln!("flick: {e}; using defaults");
        config::Config::default()
    });
    let db = config::data_dir().join("flick.db");
    let store = store::Store::open(&db)
        .unwrap_or_else(|e| panic!("flick: can't open {}: {e}", db.display()));

    if let Err(e) = hotkey::init().and_then(|()| hotkey::register(&config)) {
        eprintln!("flick: {e}");
    }
    eprintln!(
        "flick: press {} to open (Accessibility: {})",
        config.hotkey,
        if windows::is_trusted(false) { "granted" } else { "NOT granted" }
    );

    spaces::init();
    ui::init(mtm);
    app::init(config, store);
    ui::every(0.5, app::poll_clipboard);

    ns_app.run();
}

/// Another Flick.app is running (e.g. opened by hand next to the login agent's copy).
fn already_running() -> bool {
    let Some(id) = NSBundle::mainBundle().bundleIdentifier() else { return false };
    // Compare pids: a process launchd starts directly may not be registered yet itself.
    let me = std::process::id() as i32;
    NSRunningApplication::runningApplicationsWithBundleIdentifier(&id)
        .iter()
        .any(|app| app.processIdentifier() != me)
}

/// `flick snapshot <out.png> [query]`: draw the launcher to a PNG without showing it.
/// Uses the default config and an empty database, so no personal data appears.
#[expect(clippy::expect_used, reason = "the CLI entry point always runs on the main thread")]
fn snapshot(args: &[String]) {
    let Some(out) = args.first() else {
        eprintln!("usage: flick snapshot <out.png> [query]");
        std::process::exit(2);
    };
    let mtm = MainThreadMarker::new().expect("must start on the main thread");
    ui::init(mtm);
    app::init(config::Config::default(), store::Store::in_memory());
    app::set_root_query(args.get(1).map_or("", String::as_str));
    if let Err(e) = ui::snapshot(out) {
        eprintln!("flick: {e}");
        std::process::exit(1);
    }
}
