//! Table `[llm]` and its `[[llm.servers]]`. No server by default: with none, the module runs
//! nothing and every verb says how to add one. Only this Mac's config names a server; nothing
//! from a request or a reply ever becomes one.

use serde::Deserialize;

/// The longest `timeout_secs` (one hour).
pub const MAX_TIMEOUT: u64 = 3_600;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// OpenAI-compatible servers, in picker order; none by default.
    pub servers: Vec<Server>,
    /// The normal server a new chat uses (`""`: the first normal server).
    pub default_server: String,
    /// The model a new chat uses (`""`: the first one the server lists).
    pub default_model: String,
    /// Sent as the first message of every chat (`""`: none).
    pub system_prompt: String,
    /// Keep normal chats in flick.db (flick-6a0d). Private chats are never kept.
    pub history: bool,
    /// The reply's token cap sent with each request (0: the server's default).
    pub max_tokens: u32,
    /// Seconds one reply may take, start to end (1 to 3600).
    pub timeout_secs: u64,
    /// Opens the chat window (flick-6a0d); unbound by default.
    pub hotkey: Option<String>,
    /// Opens the private chat window (flick-c325); unbound by default.
    pub private_hotkey: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            servers: vec![],
            default_server: String::new(),
            default_model: String::new(),
            system_prompt: String::new(),
            history: true,
            max_tokens: 0,
            timeout_secs: 300,
            hotkey: None,
            private_hotkey: None,
        }
    }
}

/// One server: an OpenAI-compatible base URL (mlx-serve, ollama).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub name: String,
    /// Base URL, `http(s)://host:port`, with or without a trailing `/v1`.
    pub url: String,
    /// Only the private chat may use it, and the private chat may use nothing else. Your
    /// assertion that the server keeps nothing; Flick cannot check it.
    #[serde(default)]
    pub private: bool,
}

impl Server {
    /// The URL of `path` (`"models"`, `"chat/completions"`) under the server's `/v1`.
    pub fn endpoint(&self, path: &str) -> String {
        let base = self.url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        format!("{base}/v1/{path}")
    }
}

/// A URL curl gets as one argv word: http(s), a host, no whitespace or control characters.
fn good_url(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/'))
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl Settings {
    pub fn check(self) -> Result<Settings, String> {
        for (i, s) in self.servers.iter().enumerate() {
            if s.name.trim().is_empty() || s.name.chars().any(char::is_whitespace) {
                return Err(format!("[llm]: bad server name \"{}\" (one word)", s.name));
            }
            if self.servers[..i].iter().any(|o| o.name == s.name) {
                return Err(format!("[llm]: server \"{}\" is named twice", s.name));
            }
            if !good_url(&s.url) {
                return Err(format!("[llm]: server \"{}\": url must be http(s)://host[:port], not \"{}\"", s.name, s.url));
            }
        }
        if !self.default_server.is_empty() {
            match self.servers.iter().find(|s| s.name == self.default_server) {
                None => return Err(format!("[llm]: default_server \"{}\" is not in [[llm.servers]]", self.default_server)),
                Some(s) if s.private => {
                    return Err(format!("[llm]: default_server \"{}\" is private; name a normal server", s.name));
                }
                Some(_) => {}
            }
        }
        if !(1..=MAX_TIMEOUT).contains(&self.timeout_secs) {
            return Err(format!("[llm]: timeout_secs must be 1 to {MAX_TIMEOUT}"));
        }
        Ok(self)
    }

    /// The normal (not private) server called `name`, or the default one for `None`.
    pub fn normal(&self, name: Option<&str>) -> Result<&Server, String> {
        if self.servers.is_empty() {
            return Err("llm: no servers; add a [[llm.servers]] table to config.toml".into());
        }
        let default = Some(self.default_server.as_str());
        let found = match name.filter(|n| !n.is_empty()).or(default).filter(|n| !n.is_empty()) {
            Some(n) => self.servers.iter().find(|s| s.name == n).ok_or_else(|| format!("llm: no server \"{n}\""))?,
            None => self.servers.iter().find(|s| !s.private).ok_or("llm: every server is private")?,
        };
        if found.private {
            return Err(format!("llm: {} is private; only the private chat talks to it", found.name));
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    fn settings(text: &str) -> Result<Settings, String> {
        parse(text)?.section("llm")?.ok_or("disabled")?.get::<Settings>()?.check()
    }

    const TWO: &str = "[[llm.servers]]\nname = \"mlx\"\nurl = \"https://mac:11234/\"\n\
        [[llm.servers]]\nname = \"vault\"\nurl = \"https://mac:11235/v1\"\nprivate = true\n";

    #[test]
    fn the_example_documents_every_key() {
        crate::config::example::assert_documents::<Settings>("llm");
        crate::config::example::assert_documents::<Server>("llm.servers");
    }

    #[test]
    fn defaults_have_no_server() {
        let s = settings("").unwrap();
        assert_eq!(s, Settings::default());
        assert!(s.servers.is_empty());
        assert_eq!((s.history, s.max_tokens, s.timeout_secs), (true, 0, 300));
        assert_eq!((&s.hotkey, &s.private_hotkey), (&None, &None));
        assert_eq!(s.normal(None).unwrap_err(), "llm: no servers; add a [[llm.servers]] table to config.toml");
    }

    #[test]
    fn servers_resolve_to_endpoints() {
        let s = settings(TWO).unwrap();
        assert_eq!(s.servers[0].endpoint("models"), "https://mac:11234/v1/models");
        assert_eq!(s.servers[1].endpoint("chat/completions"), "https://mac:11235/v1/chat/completions");
        assert!(!s.servers[0].private && s.servers[1].private);
    }

    #[test]
    fn normal_picks_a_named_or_default_server_never_a_private_one() {
        let s = settings(TWO).unwrap();
        assert_eq!(s.normal(None).unwrap().name, "mlx");
        assert_eq!(s.normal(Some("")).unwrap().name, "mlx");
        assert_eq!(s.normal(Some("mlx")).unwrap().name, "mlx");
        assert_eq!(s.normal(Some("vault")).unwrap_err(), "llm: vault is private; only the private chat talks to it");
        assert_eq!(s.normal(Some("nope")).unwrap_err(), "llm: no server \"nope\"");
        let private_only = "[[llm.servers]]\nname = \"vault\"\nurl = \"http://h:1\"\nprivate = true\n";
        assert_eq!(settings(private_only).unwrap().normal(None).unwrap_err(), "llm: every server is private");
        let named = settings(&format!("[llm]\ndefault_server = \"mlx\"\n{TWO}")).unwrap();
        assert_eq!(named.normal(None).unwrap().name, "mlx");
    }

    #[test]
    fn bad_values_are_refused() {
        let err = |t: &str| settings(t).unwrap_err();
        let one = |name: &str, url: &str| format!("[[llm.servers]]\nname = \"{name}\"\nurl = \"{url}\"\n");
        assert_eq!(err(&one("a b", "http://h")), "[llm]: bad server name \"a b\" (one word)");
        assert_eq!(err(&one(" ", "http://h")), "[llm]: bad server name \" \" (one word)");
        assert_eq!(err(&format!("{}{}", one("a", "http://h"), one("a", "http://i"))), "[llm]: server \"a\" is named twice");
        for url in ["ftp://h", "http://", "https:///x", "http://h /x", "h:1", "-K/etc/x"] {
            let want = format!("[llm]: server \"a\": url must be http(s)://host[:port], not \"{url}\"");
            assert_eq!(err(&one("a", url)), want);
        }
        assert_eq!(err("[llm]\ndefault_server = \"x\""), "[llm]: default_server \"x\" is not in [[llm.servers]]");
        let private = format!("[llm]\ndefault_server = \"vault\"\n{TWO}");
        assert_eq!(err(&private), "[llm]: default_server \"vault\" is private; name a normal server");
        assert_eq!(err("[llm]\ntimeout_secs = 0"), "[llm]: timeout_secs must be 1 to 3600");
        assert_eq!(err("[llm]\ntimeout_secs = 3601"), "[llm]: timeout_secs must be 1 to 3600");
        assert!(err("[llm]\nnope = 1").starts_with("[llm]: unknown field `nope`"));
        assert!(err("[[llm.servers]]\nname = \"a\"\n").contains("missing field `url`"));
        assert_eq!(settings("[llm]\ntimeout_secs = 3600").unwrap().timeout_secs, 3600);
    }
}
