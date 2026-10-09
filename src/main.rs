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

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
    let store = store::Store::open(&db).unwrap_or_else(|e| panic!("flick: can't open {}: {e}", db.display()));

    if let Err(e) = hotkey::init().and_then(|()| hotkey::register(&config)) {
        eprintln!("flick: {e}");
    }
    eprintln!("flick: press {} to open", config.hotkey);

    spaces::init();
    ui::init(mtm);
    app::init(config, store);
    ui::every(0.5, app::poll_clipboard);

    ns_app.run();
}

/// Another Flick.app is running (e.g. opened by hand next to the login agent's copy).
fn already_running() -> bool {
    let Some(id) = NSBundle::mainBundle().bundleIdentifier() else { return false };
    NSRunningApplication::runningApplicationsWithBundleIdentifier(&id).count() > 1
}
