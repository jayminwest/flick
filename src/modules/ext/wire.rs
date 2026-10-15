//! The real hooks: post `ModuleChanged`, log to stderr, post a notification or a HUD toast,
//! ask whether the launcher shows, run a card `do` and Flick's own CLI.

use super::proto::Parsed;
use super::transport;
use crate::core::Event;
use crate::core::card::Do;
use crate::platform::hud::{self, Content, Corner, Options, Placement, TextCard};
use crate::platform::{events, notify, panel, pasteboard, workspace};

/// Worker thread: the inbox has an answer.
pub fn post() {
    events::post(Event::ModuleChanged { module: "ext" });
}

pub fn log(line: &str) {
    eprintln!("flick: ext: {line}");
}

/// Main thread: a banner for a failed call of `ext` (logged already).
pub fn notify(ext: &str, error: &str) {
    let _ = notify::post(&format!("ext:{ext}"), "Flick extension failed", error);
}

/// Main thread: an action's or form's line as a HUD card in the top right corner for a few
/// seconds; a newer line of the same extension replaces it.
pub fn toast(ext: &str, text: &str) {
    let card = TextCard { header: ext, time: "", context: "", body: text, link: None, pending: false };
    let placement = Placement { corner: Corner::TopRight, width: 380.0, max_cards: 4 };
    let opts = Options { timeout_secs: 6.0, sound: false, sticky: false };
    hud::show(&format!("ext:{ext}"), &Content::Text(card), &placement, &opts);
}

/// The launcher is on the screen.
pub fn visible() -> bool {
    panel::is_visible()
}

/// Main thread: a card `open_url`, `open_app` or `copy` (the others never reach here).
pub fn card(run: &Do) -> Result<(), String> {
    match run {
        Do::OpenUrl(url) => workspace::open_url(url),
        Do::OpenApp(app) => {
            let path = workspace::find_app(app).ok_or_else(|| format!("open_app: no app {app:?}"))?;
            workspace::open_file(&path);
        }
        Do::Copy(text) => pasteboard::set_text(text),
        other => return Err(format!("{} is not run here", other.to_value())),
    }
    Ok(())
}

/// Worker thread: Flick's own CLI (this binary) with `words`.
pub fn flick(words: &[String]) -> Result<Parsed, String> {
    let exe = std::env::current_exe().map_err(|e| format!("no Flick binary: {e}"))?;
    transport::flick(&exe, words)
}
