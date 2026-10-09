mod app;
#[cfg(test)]
mod characterization;
mod config;
mod core;
mod hotkey;
mod modules;
mod platform;
mod raycast;
mod root;
mod store;
mod ui;

use platform::app as macos;

#[expect(clippy::panic, reason = "startup invariant: no launcher without the database")]
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

    macos::require_main_thread();
    if macos::already_running() {
        eprintln!("flick: already running");
        return;
    }
    // No Dock icon or menu bar.
    macos::set_accessory();

    let config = config::load().unwrap_or_else(|e| {
        eprintln!("flick: {e}; using defaults");
        config::Config::default()
    });
    let db = config::data_dir().join("flick.db");
    let store = store::Store::open(&db)
        .unwrap_or_else(|e| panic!("flick: can't open {}: {e}", db.display()));

    let launcher = config.hotkey.clone();
    let hotkeys = hotkey::init();
    platform::workspace::track_recent();
    ui::init();
    app::init(config, store);
    if let Err(e) = hotkeys.and_then(|()| app::bind_hotkeys()) {
        eprintln!("flick: {e}");
    }
    eprintln!(
        "flick: press {launcher} to open (Accessibility: {})",
        if platform::ax::is_trusted(false) { "granted" } else { "NOT granted" }
    );
    platform::timer::every(0.5, app::tick);

    macos::run();
}

/// `flick snapshot <out.png> [query]`: draw the launcher to a PNG without showing it.
/// Uses the default config and an empty database, so no personal data appears.
fn snapshot(args: &[String]) {
    let Some(out) = args.first() else {
        eprintln!("usage: flick snapshot <out.png> [query]");
        std::process::exit(2);
    };
    macos::require_main_thread();
    ui::init();
    app::init(config::Config::default(), store::Store::in_memory());
    app::set_root_query(args.get(1).map_or("", String::as_str));
    if let Err(e) = ui::snapshot(out) {
        eprintln!("flick: {e}");
        std::process::exit(1);
    }
}
