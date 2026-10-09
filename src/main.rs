mod app;
#[cfg(test)]
mod characterization;
mod cli;
mod config;
mod control;
mod core;
mod hotkey;
mod modules;
mod platform;
mod raycast;
mod root;
mod ui;

use platform::app as macos;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        cli::Command::Launch => launch(),
        command => match cli::run(command) {
            0 => {}
            code => std::process::exit(code),
        },
    }
}

#[expect(clippy::panic, reason = "startup invariant: no launcher without the database")]
fn launch() {
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
    let store = core::store::Store::open(&db)
        .unwrap_or_else(|e| panic!("flick: can't open {}: {e}", db.display()));

    let hotkeys = hotkey::init();
    ui::init();
    // After init: a bad module table swaps in the default config, hotkey included.
    let launcher = app::init(config, store);
    if let Err(e) = hotkeys.and_then(|()| app::bind_hotkeys()) {
        eprintln!("flick: {e}");
    }
    eprintln!(
        "flick: press {launcher} to open (Accessibility: {})",
        if platform::ax::is_trusted(false) { "granted" } else { "NOT granted" }
    );
    platform::events::start(app::on_event);
    if let Err(e) = control::start() {
        eprintln!("flick: control socket: {e}");
    }

    macos::run();
}
