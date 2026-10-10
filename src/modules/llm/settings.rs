//! Table `[llm]` and its `[[llm.servers]]`. No server by default: with none, the module runs
//! nothing and every verb says how to add one. Only this Mac's config names a server; nothing
//! from a request or a reply ever becomes one.

use serde::Deserialize;

use super::keyfile::good_key;

/// The longest `timeout_secs` (one hour).
pub const MAX_TIMEOUT: u64 = 3_600;
/// The most `max_threads`.
pub const MAX_THREADS: usize = 10_000;

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
    /// Keep normal chats in flick.db. Private chats are never kept.
    pub history: bool,
    /// The most normal chats kept; saving one past it drops the oldest (1 to 10000).
    pub max_threads: usize,
    /// The reply's token cap sent with each request (0: the server's default).
    pub max_tokens: u32,
    /// Seconds one reply may take, start to end (1 to 3600).
    pub timeout_secs: u64,
    /// Shows or hides the chat window; unbound by default.
    pub hotkey: Option<String>,
    /// Shows or hides the private chat window; unbound by default.
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
            max_threads: 100,
            max_tokens: 0,
            timeout_secs: 300,
            hotkey: None,
            private_hotkey: None,
        }
    }
}

/// One server: an OpenAI-compatible base URL (mlx-serve, ollama).
#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub name: String,
    /// Base URL, `http(s)://host:port`, with or without a trailing `/v1`.
    pub url: String,
    /// Only the private chat may use it, and the private chat may use nothing else. Your
    /// assertion that the server keeps nothing; Flick cannot check it.
    #[serde(default)]
    pub private: bool,
    /// Sent as `Authorization: Bearer <key>` (`""`: none), through a 0600 curl config file,
    /// never curl's argv (`keyfile`).
    #[serde(default)]
    pub api_key: String,
}

/// The key never shows in a `Debug` print.
impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = if self.api_key.is_empty() { "none" } else { "set" };
        f.debug_struct("Server")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("private", &self.private)
            .field("api_key", &key)
            .finish()
    }
}

impl Server {
    /// The URL of `path` (`"models"`, `"chat/completions"`) under the server's `/v1`.
    pub fn endpoint(&self, path: &str) -> String {
        let base = self.url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        format!("{base}/v1/{path}")
    }

    /// The API key for a call, if one is set.
    pub fn key(&self) -> Option<String> {
        Some(self.api_key.clone()).filter(|k| !k.is_empty())
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
            if !s.api_key.is_empty() && !good_key(&s.api_key) {
                let why = "must be printable ASCII without spaces, quotes or backslashes";
                return Err(format!("[llm]: server \"{}\": api_key {why}", s.name));
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
        if !(1..=MAX_THREADS).contains(&self.max_threads) {
            return Err(format!("[llm]: max_threads must be 1 to {MAX_THREADS}"));
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

    /// The private server called `name`, or the first private one for `None`. Never a normal
    /// server: with no private server the private chat refuses instead of falling back.
    pub fn private(&self, name: Option<&str>) -> Result<&Server, String> {
        let mut private = self.servers.iter().filter(|s| s.private);
        let found = match name.filter(|n| !n.is_empty()) {
            None => private.next().ok_or(NO_PRIVATE)?,
            Some(n) => match self.servers.iter().find(|s| s.name == n) {
                None => return Err(format!("llm: no server \"{n}\"")),
                Some(s) if !s.private => {
                    return Err(format!("llm: {n} is not private; the private chat never uses it"));
                }
                Some(s) => s,
            },
        };
        Ok(found)
    }
}

/// Why the private chat cannot start with no private server.
pub const NO_PRIVATE: &str = "llm: no private server, so private chat is off; add a [[llm.servers]] \
    table with private = true (it never falls back to a normal server)";

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
        assert_eq!((s.history, s.max_threads, s.max_tokens, s.timeout_secs), (true, 100, 0, 300));
        assert_eq!((&s.hotkey, &s.private_hotkey), (&None, &None));
        assert_eq!(s.normal(None).unwrap_err(), "llm: no servers; add a [[llm.servers]] table to config.toml");
    }

    #[test]
    fn servers_resolve_to_endpoints() {
        let s = settings(TWO).unwrap();
        assert_eq!(s.servers[0].endpoint("models"), "https://mac:11234/v1/models");
        assert_eq!(s.servers[1].endpoint("chat/completions"), "https://mac:11235/v1/chat/completions");
        assert!(!s.servers[0].private && s.servers[1].private);
        assert_eq!(s.servers[0].key(), None);
    }

    #[test]
    fn an_api_key_is_checked_and_never_debug_printed() {
        let keyed = |key: &str| settings(&format!("[[llm.servers]]\nname = \"a\"\nurl = \"http://h\"\napi_key = \"{key}\"\n"));
        let s = keyed("sk-123").unwrap();
        assert_eq!(s.servers[0].key().as_deref(), Some("sk-123"));
        let printed = format!("{s:?}");
        assert!(!printed.contains("sk-123") && printed.contains("api_key: \"set\""), "{printed}");
        assert!(format!("{:?}", settings(TWO).unwrap()).contains("api_key: \"none\""));
        for bad in ["a b", "a\\\"b", "a\\\\b"] {
            let want = "[llm]: server \"a\": api_key must be printable ASCII without spaces, quotes or backslashes";
            assert_eq!(keyed(bad).unwrap_err(), want, "{bad}");
        }
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
    fn private_picks_only_a_private_server_and_never_falls_back() {
        let s = settings(TWO).unwrap();
        assert_eq!(s.private(None).unwrap().name, "vault");
        assert_eq!(s.private(Some("")).unwrap().name, "vault");
        assert_eq!(s.private(Some("vault")).unwrap().name, "vault");
        assert_eq!(s.private(Some("mlx")).unwrap_err(), "llm: mlx is not private; the private chat never uses it");
        assert_eq!(s.private(Some("nope")).unwrap_err(), "llm: no server \"nope\"");
        // Normal servers only, a default_server set, or none at all: refused, no fallback.
        let normal = "[llm]\ndefault_server = \"mlx\"\n[[llm.servers]]\nname = \"mlx\"\nurl = \"http://h:1\"\n";
        assert_eq!(settings(normal).unwrap().private(None).unwrap_err(), NO_PRIVATE);
        assert_eq!(settings("").unwrap().private(None).unwrap_err(), NO_PRIVATE);
        assert!(NO_PRIVATE.contains("private = true") && !NO_PRIVATE.contains("  "));
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
        assert_eq!(err("[llm]\nmax_threads = 0"), "[llm]: max_threads must be 1 to 10000");
        assert_eq!(err("[llm]\nmax_threads = 10001"), "[llm]: max_threads must be 1 to 10000");
    }
}
