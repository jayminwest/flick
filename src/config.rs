//! `~/.config/flick/config.toml`: the launcher hotkey and `[launcher]`, then one `[<module>]`
//! table per module (keyed by module id). Each module deserializes its own table; this file
//! knows no module's settings, only the legacy flat keys it maps into those tables.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

pub mod edit;
pub mod example;
mod host;
pub mod launcher;
pub mod overlay;
pub mod target;

pub use host::host_name;
pub use launcher::Launcher;

const DEFAULT_CONFIG: &str = r##"# Flick config. Edit, then run "Reload Flick Config" from Flick.
# Every option, with its default: run "flick config example".

# Modifiers: cmd, alt, ctrl, shift. Keys: Space, KeyA..KeyZ, Digit0..Digit9, F1..F12, ArrowLeft, ...
hotkey = "alt+shift+Space"

# Each module reads its own [<module>] table. "enabled = false" turns a module off:
# no items, no views, no hotkeys. Modules: app, desktop, switcher, window, quicklink,
# builtin, clip, activity, keys, herdr, task, capture, feedback, message, flick, remote.

# Switch to the most recently used app on another desktop (macOS then switches
# desktops). Backquote replaces macOS's "cycle windows of this app" shortcut.
[desktop]
# hotkey = "cmd+Backquote"

# Window switcher: fuzzy-search open windows; the first result is the previous window.
[switcher]
# hotkey = "cmd+Space"

# Global window hotkeys: <action> = "<hotkey>". Actions are the window command
# titles in kebab case: left-half, right-half, top-half, bottom-half,
# top-left-quarter, ..., first-third, center-third, last-third, first-two-thirds,
# last-two-thirds, maximize, almost-maximize, center, next-display, previous-display.
# Repeating a half cycles 1/2 -> 2/3 -> 1/3.
[window.keys]
# left-half = "ctrl+alt+ArrowLeft"
# right-half = "ctrl+alt+ArrowRight"
# maximize = "ctrl+alt+Enter"

# Key triggers. Off until set: no [keys] table means no key tap and no Caps Lock remap.
# hyper = "caps_lock" makes Caps Lock add hyper_mods to every key held with it (it then
# sends F18 until Flick quits); a quick tap sends hyper_tap. Needs Accessibility.
# A chord runs on_down when all its keys are held and on_up when one is released:
# { http = "POST http://host:port/path" } (http:// only) or { shell = "command" }.
# Keys: cmd, alt, ctrl, shift, fn, right_cmd, left_alt, ... plus one key such as KeyA.
# [keys]
# hyper = "caps_lock"
# hyper_mods = "cmd+ctrl+alt+shift"
# hyper_tap = "Escape"
# hyper_tap_ms = 300
#
# [[keys.chord]]
# name = "ptt"
# keys = ["right_cmd", "right_alt"]
# on_down = { http = "POST http://localhost:8600/pipeline/listen/start" }
# on_up = { http = "POST http://localhost:8600/pipeline/listen/stop" }

# Activity: records which app is in front as time spans in flick.db, on this Mac only.
# Off until you run "Start Activity Recording" or "flick activity on"; a menu bar dot
# shows while it records. titles = true also stores window titles (needs Accessibility).
# urls = true also stores the front tab URL of Brave, Chrome, Edge or Chromium, never from
# a private window (each browser asks once for Automation permission). Agent sessions see
# titles only with remote_titles = true, and URLs and domains only with remote_urls = true.
# exclude: bundle ids or app names never recorded; setting it replaces the default list
# (1Password, Keychain Access). Rules name a category and project at report time; app and
# title are case-insensitive regexes, and the first matching rule wins.
# [activity]
# titles = false
# urls = false
# remote_titles = false
# remote_urls = false
# exclude = ["com.1password.1password", "com.agilebits.onepassword7", "com.apple.keychainaccess"]
# hotkey = "cmd+ctrl+alt+shift+KeyR"
# merge_secs = 2
#
# [[activity.rules]]
# app = "wezterm|zed"
# project = "flick"
# category = "code"

# Herdr: coding agents in herdr on this Mac and on herdr's saved SSH machines, the ones
# waiting on you first. "Herdr Agents" in the launcher; Enter focuses the agent's pane and
# brings the terminal to the front. Read-only: Flick never sends input to an agent.
# machines = [] means local plus every enabled `herdr machine list` profile.
# remote_refresh_secs = 0 polls remote machines only while the launcher is open.
# notify: statuses that post a notification ("blocked", "done"); [] for none.
# [herdr]
# machines = ["local", "mbp-server"]
# remote_refresh_secs = 60
# terminal = "WezTerm"
# hotkey = "cmd+ctrl+alt+shift+KeyA"
# preview_lines = 6
# notify = ["blocked", "done"]

# Tasks: a task list and a timer for the one running task, in flick.db. "Start Task" in
# the launcher or "flick task start <title>"; a trailing #word sets the project. Idle,
# sleep and screen lock pause the timer. hotkey opens the task picker; unbound by default.
# [task]
# hotkey = "cmd+ctrl+alt+shift+KeyT"

# Capture: screenshots of an area, a window or a screen as PNG files in dir, copied to
# the clipboard; "Recent Captures" lists them. Needs Screen Recording (System Settings >
# Privacy & Security). save = false keeps only the clipboard copy (in a temp file); save
# and copy cannot both be off. name: {date}, {time} and {kind}. history trims rows, never
# files. colors (1 to 5, "#rrggbb" or "#rrggbbaa", keys 1-5) and width are the pens of the
# annotation editor and Draw on Screen; fade_secs > 0 fades shapes drawn on the screen.
# Hotkeys: area, window, screen, annotate, draw and cursor, all unbound by default.
# [capture]
# dir = "~/Pictures/Flick"
# name = "Flick {date} at {time}.png"
# copy = true
# save = true
# sound = true
# cursor = false
# shadow = true
# history = 200
# colors = ["#ff3b30", "#ffcc00", "#34c759", "#0a84ff", "#ffffff"]
# width = 4.0
# fade_secs = 0.0
# halo_color = "#ffcc00"
# halo_radius = 28.0
# area_hotkey = "cmd+ctrl+alt+shift+Digit4"
# window_hotkey = "cmd+ctrl+alt+shift+Digit5"
# screen_hotkey = "cmd+ctrl+alt+shift+Digit3"
# annotate_hotkey = "cmd+ctrl+alt+shift+Digit6"
# draw_hotkey = "cmd+ctrl+alt+shift+KeyD"
# cursor_hotkey = "cmd+ctrl+alt+shift+KeyC"

# Feedback: "Add Feedback…" or "fb <text>" in the launcher, or "flick feedback add <text>",
# appends one JSON line (ts, text, build, front app, query) to file. Default file:
# feedback.jsonl in the checkout this Flick was built from (git ignores it). keyword: the
# word before the text in root search. hotkey opens a feedback field; unbound by default.
# [feedback]
# file = "~/Projects/flick/feedback.jsonl"
# keyword = "fb"
# hotkey = "cmd+ctrl+alt+shift+KeyF"

# Messages: "flick message post <text>" (or "flick --host <this Mac> message post ..." from a
# peer) shows a corner card that does not take focus; Messages in the launcher lists them.
# style: panel, notification, both or none. position: top-right, top-left, bottom-right,
# bottom-left, top or bottom. timeout_secs = 0 keeps the card until dismissed. docs/message.md
# [message]
# name = "KOTA"
# style = "panel"
# position = "top-right"
# width = 380
# timeout_secs = 20
# max_cards = 4
# max_history = 50
# sound = true
# hotkey = "cmd+ctrl+alt+shift+KeyM"

# Remote: answer "flick --host <this Mac>" from these Tailscale peers, on Tailscale addresses
# only. Off until "flick remote on" or "Turn On Network Access". Peers may not reload, rebuild,
# capture, fire keys or edit config; events = true lets them stream events. docs/remote.md
# [remote]
# peers = ["mbp-server"]
# port = 7419
# events = false

# Rebuild Flick from its local checkout (no git fetch or pull; cargo runs --offline).
# source: the checkout; default: the one this app was built from. check_on_open: check
# it for newer commits when the launcher opens. gates: run scripts/check-all.sh first.
[flick]
# source = "~/Projects/flick"
# check_on_open = true
# gates = false

# Quicklinks open a URL or a path. "{query}" makes the link take an argument.
# Typing "<keyword> <text>" runs a link straight from the root search.
# app = "Safari" (a name, .app path or bundle id) opens a link in that app.
[[quicklink.links]]
name = "Google"
keyword = "g"
url = "https://www.google.com/search?q={query}"

[[quicklink.links]]
name = "GitHub Search"
keyword = "gh"
url = "https://github.com/search?q={query}&type=repositories"

[[quicklink.links]]
name = "YouTube"
keyword = "yt"
url = "https://www.youtube.com/results?search_query={query}"

[[quicklink.links]]
name = "Projects"
url = "~/Projects"
"##;

/// The flat top-level keys Flick read before per-module tables, and the `[module] key` each
/// now means. The only place that knows the old layout; old files keep working.
const LEGACY: [(&str, &str, &str); 4] = [
    ("desktop_toggle", "desktop", "hotkey"),
    ("windows_hotkey", "switcher", "hotkey"),
    ("window_keys", "window", "keys"),
    ("quicklinks", "quicklink", "links"),
];

#[derive(Debug)]
pub struct Config {
    /// The launcher hotkey. The controller owns it, not a module.
    pub hotkey: String,
    /// Every other top-level key, with legacy keys moved into their module's table.
    tables: Table,
    /// The per-host overlay merged in, if any (`overlay::load`).
    overlay: Option<PathBuf>,
}

/// One module's `[<module>]` table, without its `enabled` key.
#[derive(Debug)]
pub struct Section {
    module: String,
    table: Table,
}

impl Section {
    /// Module `module`'s section as if the file had no such table: every key at its default.
    pub fn empty(module: &str) -> Section {
        Section { module: module.into(), table: Table::new() }
    }

    /// The table as the module's own settings type. Give `T` `#[serde(default)]` so a
    /// missing table or key takes its default.
    pub fn get<T: DeserializeOwned>(&self) -> Result<T, String> {
        Value::Table(self.table.clone()).try_into().map_err(|e| format!("[{}]: {e}", self.module))
    }

    /// The table sets a key besides `enabled`. A bare `[<module>]` header counts as no table:
    /// modules that stay off until configured (kota, dictation) turn on only on a key.
    pub fn is_set(&self) -> bool {
        !self.table.is_empty()
    }
}

#[derive(Deserialize)]
struct Top {
    #[serde(default = "default_hotkey")]
    hotkey: String,
}

fn default_hotkey() -> String {
    "alt+shift+Space".into()
}

impl Default for Config {
    #[expect(
        clippy::expect_used,
        reason = "DEFAULT_CONFIG is a compile-time constant covered by tests"
    )]
    fn default() -> Self {
        parse(DEFAULT_CONFIG).expect("default config parses")
    }
}

impl Config {
    /// Module `module`'s table: `Ok(None)` when it sets `enabled = false`, an empty
    /// section when the file has no such table.
    pub fn section(&self, module: &str) -> Result<Option<Section>, String> {
        let mut table = match self.tables.get(module) {
            None => Table::new(),
            Some(Value::Table(t)) => t.clone(),
            Some(_) => return Err(format!("{module}: expected a [{module}] table")),
        };
        match table.remove("enabled") {
            None | Some(Value::Boolean(true)) => {}
            Some(Value::Boolean(false)) => return Ok(None),
            Some(_) => return Err(format!("[{module}] enabled: expected true or false")),
        }
        Ok(Some(Section { module: module.into(), table }))
    }

    /// The `[launcher]` table, checked by `parse`; the controller owns it, not a module.
    pub fn launcher(&self) -> Launcher {
        Launcher::from_tables(&self.tables).unwrap_or_default()
    }
}

/// `$FLICK_CONFIG`, else `~/.config/flick/config.toml`.
pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("FLICK_CONFIG") {
        return path.into();
    }
    dirs::home_dir().unwrap_or_default().join(".config/flick/config.toml")
}

pub fn data_dir() -> PathBuf {
    let dir = dirs::data_dir().unwrap_or_default().join("Flick");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Load the config and this Mac's overlay, writing the default file on first run.
pub fn load() -> Result<Config, String> {
    load_from(&config_path(), host_name().as_deref())
}

/// Load the config at `path` and `host`'s overlay next to it, writing the default file there
/// when it is missing.
fn load_from(path: &Path, host: Option<&str>) -> Result<Config, String> {
    if !path.exists() {
        write_default(path);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    overlay::load(path, &text, host, &Path::exists, &|p| std::fs::read_to_string(p))
}

/// Write the commented default config to `path`. Errors surface when it is read.
fn write_default(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, DEFAULT_CONFIG);
}

/// Parse config.toml text. Checks the syntax, `hotkey` and the legacy keys; module tables
/// are checked when the modules read them. Existing files must keep parsing; see
/// `crate::characterization`.
pub fn parse(text: &str) -> Result<Config, String> {
    from_table(toml::from_str(text).map_err(|e| e.to_string())?)
}

/// `parse` for an already parsed file.
fn from_table(mut tables: Table) -> Result<Config, String> {
    let Top { hotkey } = Value::Table(tables.clone()).try_into().map_err(|e| e.to_string())?;
    tables.remove("hotkey");
    for (old, module, key) in LEGACY {
        if let Some(value) = tables.remove(old) {
            map_legacy(&mut tables, old, module, key, value)?;
        }
    }
    Launcher::from_tables(&tables)?;
    Ok(Config { hotkey, tables, overlay: None })
}

/// Put legacy top-level `old = value` at `[module] key`. When both are set, arrays join
/// (the table's entries first) and tables merge; any other overlap is an error.
fn map_legacy(
    tables: &mut Table,
    old: &str,
    module: &str,
    key: &str,
    value: Value,
) -> Result<(), String> {
    let both = || format!("{old} and [{module}] {key} are both set; keep one");
    let Value::Table(table) = tables.entry(module).or_insert_with(|| Table::new().into()) else {
        return Err(format!("{module}: expected a [{module}] table"));
    };
    match (table.get_mut(key), value) {
        (None, value) => {
            table.insert(key.into(), value);
        }
        (Some(Value::Array(new)), Value::Array(old)) => new.extend(old),
        (Some(Value::Table(new)), Value::Table(old)) => {
            for (k, v) in old {
                if new.contains_key(&k) {
                    return Err(both());
                }
                new.insert(k, v);
            }
        }
        _ => return Err(both()),
    }
    Ok(())
}

#[cfg(test)]
mod tests;
