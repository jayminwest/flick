//! The front tab URL of a running browser, through `AppleScript` (`/usr/bin/osascript`). Only
//! Chromium browsers whose windows report `mode` ("normal" or "incognito") are supported, so
//! a private window is never read: anything but "normal" gives no URL.
//!
//! Safari and Arc are not supported, because Flick cannot tell their private windows:
//! - Safari's dictionary (`sdef`) has no private property on window, document or tab. The
//!   known workaround reads the label of a Window menu item through System Events: it needs
//!   Accessibility and a second Automation grant, works only while Safari is active, breaks
//!   with each localization, and races the URL read. A wrong guess records a private URL.
//! - Arc takes `incognito` only in `make new window with properties`; no source shows it as
//!   a readable window property, and its windows have no `mode`. Not verified against a real
//!   `sdef /Applications/Arc.app` (Arc was not installed), so it stays off.
//!
//! Each browser asks the user once for Automation permission (Privacy & Security >
//! Automation), naming Flick, on the first read. `front_tab_url` blocks until the browser
//! answers, and the first time until the user does: it is the one function here that must
//! run off the main thread. A read that takes longer than `LIMIT` is killed and fails, so a
//! hung browser cannot hold the worker for the two minutes an Apple Event may wait.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const OSASCRIPT: &str = "/usr/bin/osascript";

/// The longest one read may take, the user's Automation prompt included.
const LIMIT: Duration = Duration::from_secs(30);
/// How often `output_within` checks whether the child exited.
const POLL: Duration = Duration::from_millis(20);

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
    match output_within(Command::new(OSASCRIPT).args(args), LIMIT) {
        Some(out) => parse(
            out.status.success(),
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        ),
        None => TabUrl::Failed,
    }
}

/// Run `cmd` like `Command::output`, but kill it once `limit` has passed. `None` if it could
/// not start or was killed.
fn output_within(cmd: &mut Command, limit: Duration) -> Option<Output> {
    let mut child =
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    // Drain both pipes on threads, so a long URL cannot fill a pipe and stall the child.
    let (stdout, stderr) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let (stdout, stderr) = (stdout.join().unwrap_or_default(), stderr.join().unwrap_or_default());
    Some(Output { status: status?, stdout, stderr })
}

/// Read `pipe` to its end on a new thread.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
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
        // Never runs a script for a browser it cannot check for private windows.
        for bundle in
            ["com.apple.Safari", "com.apple.SafariTechnologyPreview", "company.thebrowser.Browser"]
        {
            assert!(!supports(bundle), "{bundle}");
            assert_eq!(front_tab_url(bundle), TabUrl::Failed, "{bundle}");
        }
    }

    #[test]
    fn script_skips_private_windows_and_never_launches() {
        let lines = script("com.brave.Browser");
        assert!(lines[0].starts_with("if application id \"com.brave.Browser\" is not running"));
        assert!(lines.iter().any(|l| l.contains("mode of front window) is not \"normal\"")));
        assert_eq!(lines.last().map(String::as_str), Some("end tell"));
    }

    #[test]
    fn a_read_past_its_limit_is_killed() {
        let start = Instant::now();
        assert!(
            output_within(Command::new("/bin/sleep").arg("30"), Duration::from_millis(100))
                .is_none()
        );
        assert!(start.elapsed() < Duration::from_secs(10), "killed, not waited out");
        let out = output_within(Command::new("/bin/echo").arg("https://a.dev/"), LIMIT).unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, b"https://a.dev/\n");
        // More than a pipe holds still arrives whole.
        let long =
            output_within(Command::new("/bin/sh").args(["-c", "head -c 200000 /dev/zero"]), LIMIT)
                .unwrap();
        assert_eq!(long.stdout.len(), 200_000);
        assert!(output_within(&mut Command::new("/nonexistent/osascript"), LIMIT).is_none());
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
