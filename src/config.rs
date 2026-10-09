//! `~/.config/flick/config.toml`: hotkey and quicklinks.

use std::path::PathBuf;

use serde::Deserialize;

const DEFAULT_CONFIG: &str = r#"# Flick config. Edit, then run "Reload Config" from Flick.

# Modifiers: cmd, alt, ctrl, shift. Keys: Space, KeyA..KeyZ, Digit0..Digit9, F1..F12, ...
hotkey = "alt+shift+Space"

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
    #[serde(default)]
    pub quicklinks: Vec<Quicklink>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Quicklink {
    pub name: String,
    pub url: String,
    pub keyword: Option<String>,
}

fn default_hotkey() -> String {
    "alt+shift+Space".into()
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(DEFAULT_CONFIG).expect("default config parses")
    }
}

pub fn config_path() -> PathBuf {
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

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
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
        let q = Quicklink { name: "G".into(), url: "https://x.com/?q={query}".into(), keyword: None };
        assert!(q.takes_query());
        assert_eq!(q.expand("rust lang&co"), "https://x.com/?q=rust%20lang%26co");
        let p = Quicklink { name: "P".into(), url: "/tmp".into(), keyword: None };
        assert_eq!(p.expand(""), "file:///tmp");
    }
}
