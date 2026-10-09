//! The front tab URL of a running browser, through `AppleScript` (`/usr/bin/osascript`). Only
//! Chromium browsers whose windows report `mode` ("normal" or "incognito") are supported, so
//! a private window is never read: anything but "normal" gives no URL. Safari has no way to
//! tell a private window, and Arc no `mode`, so neither is supported.
//!
//! Each browser asks the user once for Automation permission (Privacy & Security >
//! Automation), naming Flick, on the first read. `front_tab_url` blocks until the browser
//! answers, and the first time until the user does: it is the one function here that must
//! run off the main thread.

use std::process::Command;

const OSASCRIPT: &str = "/usr/bin/osascript";

/// Bundle ids whose `AppleScript` dictionary has `URL of active tab` and window `mode`.
const CHROMIUM: [&str; 8] = [
    "com.brave.Browser",
    "com.brave.Browser.beta",
    "com.brave.Browser.nightly",
    "com.google.Chrome",
    "com.google.Chrome.beta",
    "com.google.Chrome.canary",
    "com.microsoft.edgemac",
    "org.chromium.Chromium",
];

/// `errAEEventNotPermitted`: the user denied (or has not granted) Automation.
const NOT_PERMITTED: &str = "-1743";

/// What a read of the front tab found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabUrl {
    Url(String),
    /// No window, a private window, or a tab without a URL.
    None,
    /// No Automation permission for this browser.
    Denied,
    /// The script failed some other way (the browser quit, osascript is missing).
    Failed,
}

/// Flick can read the front tab URL of the browser with bundle id `bundle`.
pub fn supports(bundle: &str) -> bool {
    CHROMIUM.contains(&bundle)
}

/// The URL of the active tab in the front window of browser `bundle`. Blocks: call it from
/// a worker thread. A browser that is not running is not launched.
pub fn front_tab_url(bundle: &str) -> TabUrl {
    if !supports(bundle) {
        return TabUrl::Failed;
    }
    let args: Vec<String> = script(bundle).into_iter().flat_map(|l| ["-e".to_owned(), l]).collect();
    match Command::new(OSASCRIPT).args(args).output() {
        Ok(out) => parse(
            out.status.success(),
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        ),
        Err(_) => TabUrl::Failed,
    }
}

/// The script's lines for Chromium browser `bundle` (one of `CHROMIUM`, never user text).
fn script(bundle: &str) -> Vec<String> {
    [
        &format!("if application id \"{bundle}\" is not running then return \"\""),
        &format!("tell application id \"{bundle}\""),
        "if (count of windows) is 0 then return \"\"",
        "if (mode of front window) is not \"normal\" then return \"\"",
        "return URL of active tab of front window",
        "end tell",
    ]
    .map(ToOwned::to_owned)
    .to_vec()
}

/// osascript's result: exit status, stdout and stderr.
fn parse(ok: bool, stdout: &str, stderr: &str) -> TabUrl {
    match stdout.trim() {
        _ if !ok && stderr.contains(NOT_PERMITTED) => TabUrl::Denied,
        _ if !ok => TabUrl::Failed,
        "" | "missing value" => TabUrl::None,
        url => TabUrl::Url(url.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_chromium_browsers_only() {
        assert!(supports("com.brave.Browser") && supports("com.google.Chrome"));
        assert!(!supports("com.apple.Safari") && !supports("company.thebrowser.Browser"));
        // Never runs a script for a browser it cannot check for private windows.
        assert_eq!(front_tab_url("com.apple.Safari"), TabUrl::Failed);
    }

    #[test]
    fn script_skips_private_windows_and_never_launches() {
        let lines = script("com.brave.Browser");
        assert!(lines[0].starts_with("if application id \"com.brave.Browser\" is not running"));
        assert!(lines.iter().any(|l| l.contains("mode of front window) is not \"normal\"")));
        assert_eq!(lines.last().map(String::as_str), Some("end tell"));
    }

    #[test]
    fn output_maps_to_a_result() {
        let url = TabUrl::Url("https://a.dev/x".into());
        assert_eq!(parse(true, "https://a.dev/x\n", ""), url);
        assert_eq!(parse(true, "\n", ""), TabUrl::None);
        assert_eq!(parse(true, "missing value\n", ""), TabUrl::None);
        let denied =
            "execution error: Not authorized to send Apple events to Brave Browser. (-1743)";
        assert_eq!(parse(false, "", denied), TabUrl::Denied);
        assert_eq!(parse(false, "", "execution error: (-600)"), TabUrl::Failed);
    }
}
