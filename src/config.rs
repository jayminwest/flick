//! `~/.config/flick/config.toml`: the launcher hotkey, then one `[<module>]` table per
//! module (keyed by module id). Each module deserializes its own table; this file knows no
//! module's settings, only the legacy flat keys it maps into those tables.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

pub mod edit;

const DEFAULT_CONFIG: &str = r#"# Flick config. Edit, then run "Reload Flick Config" from Flick.

# Modifiers: cmd, alt, ctrl, shift. Keys: Space, KeyA..KeyZ, Digit0..Digit9, F1..F12, ArrowLeft, ...
hotkey = "alt+shift+Space"

# Each module reads its own [<module>] table. "enabled = false" turns a module off:
# no items, no views, no hotkeys. Modules: app, desktop, switcher, window, quicklink,
# builtin, clip.

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

# Quicklinks open a URL or a path. "{query}" makes the link take an argument.
# Typing "<keyword> <text>" runs a link straight from the root search.
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
"#;

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
}

/// One module's `[<module>]` table, without its `enabled` key.
#[derive(Debug)]
pub struct Section {
    module: String,
    table: Table,
}

impl Section {
    /// The table as the module's own settings type. Give `T` `#[serde(default)]` so a
    /// missing table or key takes its default.
    pub fn get<T: DeserializeOwned>(&self) -> Result<T, String> {
        Value::Table(self.table.clone()).try_into().map_err(|e| format!("[{}]: {e}", self.module))
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

/// Load the config, writing the default file on first run.
pub fn load() -> Result<Config, String> {
    let path = config_path();
    if !path.exists() {
        write_default(&path);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
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
    let mut tables: Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let Top { hotkey } = Value::Table(tables.clone()).try_into().map_err(|e| e.to_string())?;
    tables.remove("hotkey");
    for (old, module, key) in LEGACY {
        if let Some(value) = tables.remove(old) {
            map_legacy(&mut tables, old, module, key, value)?;
        }
    }
    Ok(Config { hotkey, tables })
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
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn keys(c: &Config) -> BTreeMap<String, String> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct W {
            keys: BTreeMap<String, String>,
        }
        c.section("window").unwrap().unwrap().get::<W>().unwrap().keys
    }

    fn links(c: &Config) -> Vec<String> {
        #[derive(Deserialize)]
        struct L {
            name: String,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Q {
            links: Vec<L>,
        }
        let q: Q = c.section("quicklink").unwrap().unwrap().get().unwrap();
        q.links.into_iter().map(|l| l.name).collect()
    }

    #[test]
    fn default_config_parses() {
        let c = Config::default();
        assert_eq!(c.hotkey, "alt+shift+Space");
        assert_eq!(links(&c), ["Google", "GitHub Search", "YouTube", "Projects"]);
        assert!(keys(&c).is_empty());
    }

    #[test]
    fn legacy_keys_move_into_module_tables() {
        let c = parse("desktop_toggle = \"cmd+K\"\n[window_keys]\ncenter = \"ctrl+C\"").unwrap();
        let desktop = c.section("desktop").unwrap().unwrap();
        assert_eq!(desktop.get::<Table>().unwrap().get("hotkey").unwrap().as_str(), Some("cmd+K"));
        assert_eq!(keys(&c).get("center").map(String::as_str), Some("ctrl+C"));
        assert!(c.section("desktop_toggle").unwrap().unwrap().get::<Table>().unwrap().is_empty());
    }

    #[test]
    fn legacy_and_module_keys_combine() {
        let text = "[[quicklinks]]\nname = \"Old\"\nurl = \"/\"\n\
                    [[quicklink.links]]\nname = \"New\"\nurl = \"/\"\n\
                    [window_keys]\ncenter = \"ctrl+C\"\n[window.keys]\nmaximize = \"ctrl+M\"";
        let c = parse(text).unwrap();
        assert_eq!(links(&c), ["New", "Old"]);
        assert_eq!(keys(&c).len(), 2);
        let err = parse("windows_hotkey = \"a\"\n[switcher]\nhotkey = \"b\"").unwrap_err();
        assert_eq!(err, "windows_hotkey and [switcher] hotkey are both set; keep one");
        let err = parse("[window_keys]\nc = \"a\"\n[window.keys]\nc = \"b\"").unwrap_err();
        assert!(err.starts_with("window_keys and [window] keys"), "{err}");
        assert!(parse("window = 1\n[window_keys]\nc = \"a\"").is_err());
    }

    #[test]
    fn enabled_flag_turns_a_module_off() {
        let c = parse("window = 2\n[clip]\nenabled = false\n[app]\nenabled = true\nx = 1").unwrap();
        assert!(c.section("clip").unwrap().is_none());
        let app = c.section("app").unwrap().unwrap().get::<Table>().unwrap();
        assert_eq!(app.keys().collect::<Vec<_>>(), ["x"]);
        assert!(c.section("nothing").unwrap().is_some());
        assert_eq!(c.section("window").unwrap_err(), "window: expected a [window] table");
        let c = parse("[clip]\nenabled = \"no\"").unwrap();
        assert_eq!(c.section("clip").unwrap_err(), "[clip] enabled: expected true or false");
    }

    #[test]
    fn section_errors_name_the_table() {
        #[derive(Deserialize, Debug)]
        struct S {
            #[expect(dead_code, reason = "only its absence is tested")]
            need: String,
        }
        let c = parse("[desktop]").unwrap();
        let err = c.section("desktop").unwrap().unwrap().get::<S>().unwrap_err();
        assert!(err.starts_with("[desktop]: missing field `need`"), "{err}");
    }
}
