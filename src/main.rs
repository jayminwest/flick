mod app;
mod apps;
mod config;
mod hotkey;
mod search;
mod store;
mod ui;
mod windows;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

fn main() {
    let mtm = MainThreadMarker::new().expect("must start on the main thread");
    let ns_app = NSApplication::sharedApplication(mtm);
    // No Dock icon or menu bar.
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let config = config::load().unwrap_or_else(|e| {
        eprintln!("flick: {e}; using defaults");
        config::Config::default()
    });
    let db = config::data_dir().join("flick.db");
    let store = store::Store::open(&db).unwrap_or_else(|e| panic!("flick: can't open {}: {e}", db.display()));

    if let Err(e) = hotkey::init().and_then(|()| hotkey::register(&config.hotkey)) {
        eprintln!("flick: {e}");
    }
    eprintln!("flick: press {} to open", config.hotkey);

    ui::init(mtm);
    app::init(config, store);
    ui::every(0.5, app::poll_clipboard);

    ns_app.run();
}
