//! The driven instance (`flick --drive`): the real launcher for an agent to drive and read
//! over a scratch control socket, with nothing on screen and nothing of the user's touched.
//!
//! It boots as `flick snapshot` does: the panel never shows, the store is in memory, and
//! there are no hotkeys, key tap, status items, platform observers or already-running check.
//! It reads its fixture config from `$FLICK_CONFIG` and serves the normal control socket on
//! `$FLICK_SOCKET`; it needs both, so it never reads the user's config (a `reload` included)
//! or takes the user's socket. `modules::DRIVEN_OFF` modules stay out.
//!
//! The drive verbs (`drive key|type|field|scene|shot|wait`, `control`) are control requests
//! that only a driven instance answers. They enter through the seams real input uses
//! (`app::command`, `app::query_changed`, `app::field_changed`) and read what the panel was
//! last handed (`ui::scene`, `ui::snapshot`).

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::core::later::{self, Answer};
use crate::core::store::Store;
use crate::platform::panel::Key;
use crate::platform::{app as macos, events, timer};
use crate::{app, config, control, ui};

/// The answer to a drive verb outside a driven instance.
pub const NOT_DRIVEN: &str = "drive: only a driven instance answers (flick --drive)";

const USAGE: &str = "usage: drive key <name> | type <text> | field <n|label> <text> | scene | shot <abs.png> | wait [ms]";

/// `drive key` names, as the panel's keys.
const KEYS: [(&str, Key); 16] = [
    ("up", Key::Up),
    ("down", Key::Down),
    ("enter", Key::Enter),
    ("tab", Key::Tab),
    ("shift-tab", Key::BackTab),
    ("escape", Key::Escape),
    ("backspace", Key::Backspace),
    ("cmd-k", Key::CmdK),
    ("cmd-q", Key::CmdQ),
    ("cmd-enter", Key::CmdEnter),
    ("line-up", Key::LineUp),
    ("line-down", Key::LineDown),
    ("page-up", Key::PageUp),
    ("page-down", Key::PageDown),
    ("top", Key::Top),
    ("bottom", Key::Bottom),
];

/// What `drive key` answers for a name not in `KEYS`.
const KEY_NAMES: &str = "one of up, down, enter, tab, shift-tab, escape, backspace, cmd-k, cmd-q, cmd-enter, line-up, line-down, page-up, page-down, top, bottom";

/// The scene of a hidden launcher.
const HIDDEN: &str = "screen: hidden\n";
/// How long `drive wait` waits by default, and at most, in milliseconds.
const WAIT_MS: u64 = 5_000;
const MAX_WAIT_MS: u64 = 60_000;
/// How often `drive wait` looks at the scene.
const POLL_SECS: f64 = 0.025;

thread_local! {
    /// The scene as the last drive verb left it: `drive wait` waits for another.
    static SEEN: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Run a driven instance until it quits. Returns the exit code when it cannot start.
pub fn launch() -> i32 {
    if let Err(e) = scratch(std::env::var_os("FLICK_CONFIG"), std::env::var_os("FLICK_SOCKET")) {
        eprintln!("flick: {e}");
        return 2;
    }
    macos::require_main_thread();
    let socket = control::socket_path();
    if control::running() {
        eprintln!("flick: a Flick already answers on {}", socket.display());
        return 1;
    }
    // Before any window exists.
    macos::set_driven();
    macos::set_accessory();
    let config = config::load().unwrap_or_else(|e| {
        eprintln!("flick: {e}; using defaults");
        config::Config::default()
    });
    ui::init();
    app::init(config, Store::in_memory());
    events::start_posted(app::on_event);
    if let Err(e) = control::start() {
        eprintln!("flick: control socket: {e}");
        return 1;
    }
    eprintln!("flick: driven on {}", socket.display());
    macos::run();
    0
}

/// `Err` unless both the config and the socket are set to paths of their own.
fn scratch(config: Option<OsString>, socket: Option<OsString>) -> Result<(), &'static str> {
    let set = |v: Option<OsString>| v.is_some_and(|v| !v.is_empty());
    if set(config) && set(socket) {
        Ok(())
    } else {
        Err("--drive needs FLICK_CONFIG and FLICK_SOCKET set to scratch paths")
    }
}

/// A drive request: `args` are the words after `drive`. Runs on the main thread, with the
/// controller's state free. Input verbs answer `taken` when the launcher handled them and
/// `passed` when not (the real panel would then do its default); a hidden launcher opens on
/// root search first, as the launcher hotkey would.
pub fn control(args: &[String]) -> Answer {
    if !macos::is_driven() {
        return Err(NOT_DRIVEN.into());
    }
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let answer = match words.as_slice() {
        ["key", name] => key(name),
        ["type", text @ ..] if !text.is_empty() => type_text(&text.join(" ")),
        ["field", which, text @ ..] => set_field(which, &text.join(" ")),
        ["scene"] => Ok(scene()),
        ["shot", path] => shot(path),
        ["wait"] => return Ok(wait(WAIT_MS)),
        ["wait", ms] => return wait_ms(ms).map(wait),
        _ => Err(USAGE.into()),
    };
    SEEN.set(scene());
    answer
}

/// What the launcher shows (`ui::scene`), or `screen: hidden`.
fn scene() -> String {
    if ui::is_visible() { ui::scene() } else { HIDDEN.into() }
}

fn taken(handled: bool) -> String {
    (if handled { "taken" } else { "passed" }).into()
}

/// Open root search if the launcher is hidden.
fn open() {
    if !ui::is_visible() {
        app::toggle();
    }
}

fn key(name: &str) -> Answer {
    let key = key_named(name)?;
    open();
    Ok(taken(app::command(key)))
}

fn key_named(name: &str) -> Result<Key, String> {
    KEYS.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|&(_, key)| key)
        .ok_or_else(|| format!("drive key: no key \"{name}\" ({KEY_NAMES})"))
}

/// Type `text` at the end of the focused form field, else of the search field. A screen
/// with a title in place of the search field takes no text.
fn type_text(text: &str) -> Answer {
    open();
    if let Some((_, focused)) = app::form_fields() {
        ui::set_field(focused, &(ui::field_value(focused) + text));
        app::field_changed(focused);
        app::redraw();
    } else if searchable(&ui::scene()) {
        ui::set_search(&(ui::query() + text));
        app::query_changed();
    } else {
        return Err("drive type: this screen has no search field".into());
    }
    Ok(taken(true))
}

/// Whether `scene` is a list screen with a search field (no title).
fn searchable(scene: &str) -> bool {
    scene.lines().nth(1).is_some_and(|line| line.starts_with("query:"))
}

/// Replace the text of the form field `which` (a 1-based number or a label).
fn set_field(which: &str, text: &str) -> Answer {
    let (labels, _) = app::form_fields().ok_or("drive field: no form on the panel")?;
    let index = field_index(&labels, which)?;
    ui::set_field(index, text);
    app::field_changed(index);
    app::redraw();
    Ok(taken(true))
}

/// The index of field `which` among `labels`: a 1-based number, else a label (any case).
fn field_index(labels: &[String], which: &str) -> Result<usize, String> {
    let by_number = which.parse::<usize>().ok().and_then(|n| n.checked_sub(1));
    by_number
        .filter(|&i| i < labels.len().min(ui::FORM_FIELDS))
        .or_else(|| labels.iter().position(|l| l.eq_ignore_ascii_case(which)))
        .filter(|&i| i < ui::FORM_FIELDS)
        .ok_or_else(|| format!("drive field: no field \"{which}\" (fields: {})", labels.join(", ")))
}

fn shot(path: &str) -> Answer {
    if !path.starts_with('/') {
        return Err("drive shot: give an absolute path".into());
    }
    ui::snapshot(path).map(|()| format!("Wrote {path}"))
}

fn wait_ms(ms: &str) -> Result<u64, String> {
    let ms: u64 = ms.parse().map_err(|_| format!("drive wait: bad milliseconds \"{ms}\""))?;
    Ok(ms.min(MAX_WAIT_MS))
}

/// Answer with the scene once it differs from the one the last drive verb left (at once if
/// it already does), or an error after `ms`. The main thread stays free meanwhile.
fn wait(ms: u64) -> String {
    let now = scene();
    if SEEN.with_borrow(|seen| *seen != now) {
        SEEN.set(now.clone());
        return now;
    }
    let max = Duration::from_millis(ms);
    let tx = later::answer_within(max + Duration::from_secs(1));
    watch(tx, Instant::now() + max, ms);
    now
}

fn watch(tx: Sender<Answer>, deadline: Instant, ms: u64) {
    let tx = Cell::new(Some(tx));
    timer::after(POLL_SECS, move || {
        let Some(tx) = tx.take() else { return };
        let now = scene();
        if SEEN.with_borrow(|seen| *seen != now) {
            SEEN.set(now.clone());
            let _ = tx.send(Ok(now));
        } else if Instant::now() >= deadline {
            let _ = tx.send(Err(format!("drive wait: no change in {ms} ms")));
        } else {
            watch(tx, deadline, ms);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_driven_instance_needs_a_scratch_config_and_socket() {
        let path = |s: &str| Some(OsString::from(s));
        assert!(scratch(path("/tmp/x/config.toml"), path("/tmp/x/flick.sock")).is_ok());
        for (config, socket) in [
            (None, path("/tmp/x/flick.sock")),
            (path("/tmp/x/config.toml"), None),
            (path(""), path("/tmp/x/flick.sock")),
            (path("/tmp/x/config.toml"), path("")),
            (None, None),
        ] {
            assert!(scratch(config.clone(), socket.clone()).is_err(), "{config:?} {socket:?}");
        }
    }

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn only_a_driven_instance_answers_drive_verbs() {
        for verb in [&["scene"][..], &["key", "enter"], &["type", "x"], &["wait"], &[]] {
            assert_eq!(control(&words(verb)), Err(NOT_DRIVEN.into()), "{verb:?}");
        }
    }

    #[test]
    fn keys_go_by_name_in_any_case() {
        assert_eq!(key_named("enter"), Ok(Key::Enter));
        assert_eq!(key_named("Shift-Tab"), Ok(Key::BackTab));
        assert_eq!(key_named("CMD-K"), Ok(Key::CmdK));
        let err = key_named("f5").unwrap_err();
        assert!(err.starts_with("drive key: no key \"f5\" (one of up, down, enter,"), "{err}");
        assert!(KEYS.iter().all(|(n, k)| key_named(n) == Ok(*k) && KEY_NAMES.contains(n)));
    }

    #[test]
    fn only_a_list_without_a_title_takes_typed_text() {
        assert!(searchable("screen: root\nquery: \nrows: 1-1 of 1\n"));
        assert!(searchable("screen: actions\nquery: co\nempty: No Actions\n"));
        assert!(!searchable("screen: confirm\ntitle: Delete?\n"));
        assert!(!searchable("screen: list\ntitle: Log\n"));
        assert!(!searchable(HIDDEN));
        assert!(!searchable(""));
    }

    #[test]
    fn fields_go_by_number_or_label() {
        let labels = words(&["Name", "URL", "2"]);
        assert_eq!(field_index(&labels, "1"), Ok(0));
        assert_eq!(field_index(&labels, "url"), Ok(1));
        // A number past the fields may still be a label.
        assert_eq!(field_index(&words(&["A", "9"]), "9"), Ok(1));
        assert_eq!(
            field_index(&labels, "0"),
            Err("drive field: no field \"0\" (fields: Name, URL, 2)".into())
        );
        assert!(field_index(&labels, "Title").is_err());
        assert!(field_index(&labels, "4").is_err());
        // Only the fields the panel can show take text.
        let many: Vec<String> = (1..=8).map(|n| format!("F{n}")).collect();
        assert_eq!(field_index(&many, "6"), Ok(5));
        assert!(field_index(&many, "7").is_err());
        assert!(field_index(&many, "F7").is_err());
    }

    #[test]
    fn waits_take_milliseconds_up_to_a_minute() {
        assert_eq!(wait_ms("250"), Ok(250));
        assert_eq!(wait_ms("600000"), Ok(MAX_WAIT_MS));
        assert_eq!(wait_ms("soon"), Err("drive wait: bad milliseconds \"soon\"".into()));
    }
}
