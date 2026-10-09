//! `~/.config/flick/config.toml`: hotkey and quicklinks.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_CONFIG: &str = r#"# Flick config. Edit, then run "Reload Flick Config" from Flick.

# Modifiers: cmd, alt, ctrl, shift. Keys: Space, KeyA..KeyZ, Digit0..Digit9, F1..F12, ArrowLeft, ...
hotkey = "alt+shift+Space"

# Switch to the most recently used app on another desktop (macOS then switches
# desktops). Backquote replaces macOS's "cycle windows of this app" shortcut.
# desktop_toggle = "cmd+Backquote"

# Window switcher: fuzzy-search open windows; the first result is the previous window.
# windows_hotkey = "cmd+Space"

# Global window hotkeys: <action> = "<hotkey>". Actions are the window command
# titles in kebab case: left-half, right-half, top-half, bottom-half,
# top-left-quarter, ..., first-third, center-third, last-third, first-two-thirds,
# last-two-thirds, maximize, almost-maximize, center, next-display, previous-display.
# Repeating a half cycles 1/2 -> 2/3 -> 1/3.
[window_keys]
# left-half = "ctrl+alt+ArrowLeft"
# right-half = "ctrl+alt+ArrowRight"
# maximize = "ctrl+alt+Enter"

# Quicklinks open a URL or a path. "{query}" makes the link take an argument.
# Typing "<keyword> <text>" runs a link straight from the root search.
[[quicklinks]]
name = "Google"
keyword = "g"
url = "https://www.google.com/search?q={query}"

[[quicklinks]]
name = "GitHub Search"
keyword = "gh"
url = "https://github.com/search?q={query}&type=repositories"

[[quicklinks]]
name = "YouTube"
keyword = "yt"
url = "https://www.youtube.com/results?search_query={query}"

[[quicklinks]]
name = "Projects"
url = "~/Projects"
"#;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// Hotkey that switches to the most recent app on another desktop.
    pub desktop_toggle: Option<String>,
    /// Hotkey that opens the window switcher.
    pub windows_hotkey: Option<String>,
    #[serde(default)]
    pub window_keys: BTreeMap<String, String>,
    #[serde(default)]
    pub quicklinks: Vec<Quicklink>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Quicklink {
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
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
        toml::from_str(DEFAULT_CONFIG).expect("default config parses")
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
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, DEFAULT_CONFIG);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

impl Quicklink {
    pub fn takes_query(&self) -> bool {
        self.url.contains("{query}")
    }

    /// The URL to open. Paths (`/…`, `~/…`) become file URLs.
    pub fn expand(&self, query: &str) -> String {
        let url = self.url.replace("{query}", &percent_encode(query));
        if let Some(rest) = url.strip_prefix("~/") {
            format!("file://{}", dirs::home_dir().unwrap_or_default().join(rest).display())
        } else if url.starts_with('/') {
            format!("file://{url}")
        } else {
            url
        }
    }
}

#[expect(clippy::format_push_string, reason = "pre-gate code; behavior frozen until flick-ea94")]
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses() {
        let c = Config::default();
        assert_eq!(c.hotkey, "alt+shift+Space");
        assert!(c.quicklinks.iter().any(|q| q.keyword.as_deref() == Some("g")));
    }

    #[test]
    fn expands_query_and_paths() {
        let q =
            Quicklink { name: "G".into(), url: "https://x.com/?q={query}".into(), keyword: None };
        assert!(q.takes_query());
        assert_eq!(q.expand("rust lang&co"), "https://x.com/?q=rust%20lang%26co");
        let p = Quicklink { name: "P".into(), url: "/tmp".into(), keyword: None };
        assert_eq!(p.expand(""), "file:///tmp");
    }
}
