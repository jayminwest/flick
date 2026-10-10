//! Table `[kota]`. Defaults fit Jaymin's setup: KOTA runs in a herdr pane on mbp-server,
//! kota-dash serves `/ok` on its tailnet name.

use serde::Deserialize;

use super::presence::FAST_SECS;

/// The `machine` that means this Mac's own herdr server (no `--machine`).
pub const LOCAL: &str = "local";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// herdr's saved machine that runs KOTA; `local`, or this Mac's host name, asks the
    /// herdr server on this Mac.
    pub machine: String,
    /// KOTA's working directory on that machine: the pane is agent `claude` there.
    pub cwd: String,
    /// The pane name that wins when several panes match (`""`: none).
    pub pane: String,
    /// The herdr CLI.
    pub herdr: String,
    /// kota-dash's base URL; presence reads `<dash>/ok`.
    pub dash: String,
    /// The ssh target for quick ask (flick-039d).
    pub ssh: String,
    /// `kota-ask` on that machine, relative to its home (flick-039d).
    pub kota_ask: String,
    /// Seconds between rounds; 0: only on demand.
    pub poll_secs: u64,
    /// Seconds of fast polling after an ask (flick-039d).
    pub fast_secs: u64,
    /// Show the menu bar item (flick-78b9).
    pub status_item: bool,
    /// Notify when KOTA goes down (flick-78b9).
    pub notify_down: bool,
    /// Opens Ask KOTA (flick-039d); unbound by default.
    pub hotkey: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            machine: "mbp-server".into(),
            cwd: "/Users/jaymin/kota".into(),
            pane: String::new(),
            herdr: "herdr".into(),
            dash: "https://mbp-server.tail1b7f44.ts.net:8310".into(),
            ssh: "jaymin@mbp-server".into(),
            kota_ask: ".dotfiles/home/.local/bin/kota-ask".into(),
            poll_secs: 60,
            fast_secs: 180,
            status_item: true,
            notify_down: true,
            hotkey: None,
        }
    }
}

impl Settings {
    pub fn check(self) -> Result<Settings, String> {
        let blank = |v: &str| v.trim().is_empty();
        if blank(&self.machine) || self.machine.contains(char::is_whitespace) {
            return Err(format!("[kota]: bad machine \"{}\"", self.machine));
        }
        if blank(&self.cwd) || blank(&self.herdr) {
            return Err("[kota]: cwd and herdr must not be empty".into());
        }
        if !(self.dash.starts_with("http://") || self.dash.starts_with("https://")) {
            return Err(format!("[kota]: dash must be an http(s) URL, not \"{}\"", self.dash));
        }
        if (1..FAST_SECS).contains(&self.poll_secs) {
            return Err(format!("[kota]: poll_secs must be 0 or at least {FAST_SECS}"));
        }
        Ok(self)
    }

    /// Whether `machine` is this Mac's server, given this Mac's short host name.
    pub fn is_local(&self, host: Option<&str>) -> bool {
        self.machine == LOCAL || host.is_some_and(|h| self.machine.eq_ignore_ascii_case(h))
    }

    /// kota-dash's `/ok` URL.
    pub fn ok_url(&self) -> String {
        format!("{}/ok", self.dash.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    fn settings(text: &str) -> Result<Settings, String> {
        parse(text)?.section("kota")?.ok_or("disabled")?.get::<Settings>()?.check()
    }

    #[test]
    fn the_example_documents_every_key() {
        crate::config::example::assert_documents::<Settings>("kota");
    }

    #[test]
    fn defaults_fit_the_kota_server() {
        let s = settings("").unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!((s.machine.as_str(), s.cwd.as_str(), s.pane.as_str()), ("mbp-server", "/Users/jaymin/kota", ""));
        assert_eq!((s.ssh.as_str(), s.kota_ask.as_str()), ("jaymin@mbp-server", ".dotfiles/home/.local/bin/kota-ask"));
        assert_eq!((s.poll_secs, s.fast_secs, s.status_item, s.notify_down), (60, 180, true, true));
        assert_eq!(s.hotkey, None);
        assert_eq!(s.ok_url(), "https://mbp-server.tail1b7f44.ts.net:8310/ok");
        assert_eq!(settings("[kota]\ndash = \"http://h:1/\"").unwrap().ok_url(), "http://h:1/ok");
    }

    #[test]
    fn bad_values_are_refused() {
        let err = |t: &str| settings(t).unwrap_err();
        assert_eq!(err("[kota]\nmachine = \"a b\""), "[kota]: bad machine \"a b\"");
        assert_eq!(err("[kota]\nmachine = \"\""), "[kota]: bad machine \"\"");
        assert_eq!(err("[kota]\ncwd = \" \""), "[kota]: cwd and herdr must not be empty");
        assert_eq!(err("[kota]\nherdr = \"\""), "[kota]: cwd and herdr must not be empty");
        assert_eq!(err("[kota]\ndash = \"ftp://x\""), "[kota]: dash must be an http(s) URL, not \"ftp://x\"");
        assert_eq!(err("[kota]\npoll_secs = 5"), "[kota]: poll_secs must be 0 or at least 15");
        assert!(err("[kota]\nnope = 1").starts_with("[kota]: unknown field `nope`"));
        assert_eq!(settings("[kota]\npoll_secs = 0").unwrap().poll_secs, 0);
        assert_eq!(settings("[kota]\npoll_secs = 15").unwrap().poll_secs, 15);
    }

    #[test]
    fn local_means_this_macs_server() {
        let s = |m: &str| Settings { machine: m.into(), ..Settings::default() };
        assert!(s("local").is_local(None));
        assert!(s("MBP-Server").is_local(Some("mbp-server")));
        assert!(!s("mbp-server").is_local(Some("jaymins-macbook-pro")));
        assert!(!s("mbp-server").is_local(None));
    }
}
