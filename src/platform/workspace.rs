//! Running apps, opening files and URLs (`NSWorkspace`). Activation events come from
//! `super::events`.

use std::path::{Path, PathBuf};

use objc2_app_kit::{
    NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{NSArray, NSString, NSURL};

/// Folders searched for an app given by name, in order.
const APP_DIRS: [&str; 5] = [
    "/Applications",
    "/System/Applications",
    "/Applications/Utilities",
    "/System/Applications/Utilities",
    "/System/Library/CoreServices",
];

/// Open `url` in its default app.
pub fn open_url(url: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

/// The app bundle that `app` names: a `.app` path (`~/` allowed), an app name looked up in the
/// standard folders and `~/Applications` ("Safari"), or a bundle id ("com.apple.Safari").
/// `None` when nothing matches.
pub fn find_app(app: &str) -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    if let Some(found) = app_candidates(app, &home).into_iter().find(|p| p.is_dir()) {
        return Some(found);
    }
    let app = app.trim();
    if !app.contains('.') || app.contains('/') {
        return None;
    }
    let ws = NSWorkspace::sharedWorkspace();
    let url = ws.URLForApplicationWithBundleIdentifier(&NSString::from_str(app))?;
    url.path().map(|p| PathBuf::from(p.to_string()))
}

/// Where app `app` could be, most specific first: the path itself when it is one, else
/// `<dir>/<app>.app` for each app folder.
fn app_candidates(app: &str, home: &Path) -> Vec<PathBuf> {
    let app = app.trim();
    if app.is_empty() {
        return vec![];
    }
    if let Some(rest) = app.strip_prefix("~/") {
        return vec![home.join(rest)];
    }
    if app.starts_with('/') {
        return vec![PathBuf::from(app)];
    }
    let is_bundle = Path::new(app).extension().is_some_and(|e| e.eq_ignore_ascii_case("app"));
    let bundle = if is_bundle { app.to_string() } else { format!("{app}.app") };
    let user = home.join("Applications");
    APP_DIRS.iter().map(PathBuf::from).chain([user]).map(|dir| dir.join(&bundle)).collect()
}

/// Open `url` with the app bundle at `app` (from `find_app`) instead of the default app.
pub fn open_url_with(url: &str, app: &Path) {
    let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else { return };
    let app = NSURL::fileURLWithPath(&NSString::from_str(&app.display().to_string()));
    NSWorkspace::sharedWorkspace().openURLs_withApplicationAtURL_configuration_completionHandler(
        &NSArray::from_retained_slice(&[url]),
        &app,
        &NSWorkspaceOpenConfiguration::configuration(),
        None,
    );
}

/// Open the file or app bundle at `path`.
pub fn open_file(path: &Path) {
    NSWorkspace::sharedWorkspace()
        .openURL(&NSURL::fileURLWithPath(&NSString::from_str(&path.display().to_string())));
}

pub fn frontmost_pid() -> Option<i32> {
    NSWorkspace::sharedWorkspace().frontmostApplication().map(|a| a.processIdentifier())
}

/// App `pid` is a regular (Dock) app, not a menu-bar or background one.
pub fn is_regular(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .is_some_and(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular)
}

/// Pids of running apps that are not hidden, in the system's order.
pub fn unhidden_app_pids() -> Vec<i32> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|a| !a.isHidden())
        .map(|a| a.processIdentifier())
        .collect()
}

/// A running regular (Dock) app.
pub struct RunningApp {
    pub pid: i32,
    pub name: String,
    pub bundle: Option<PathBuf>,
    pub hidden: bool,
}

/// Running regular apps other than `except`, in the system's order.
pub fn regular_apps(except: i32) -> Vec<RunningApp> {
    let mut out = Vec::new();
    for app in &NSWorkspace::sharedWorkspace().runningApplications() {
        let pid = app.processIdentifier();
        if pid == except || app.activationPolicy() != NSApplicationActivationPolicy::Regular {
            continue;
        }
        out.push(RunningApp {
            pid,
            name: app.localizedName().map(|n| n.to_string()).unwrap_or_default(),
            bundle: app.bundleURL().and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string())),
            hidden: app.isHidden(),
        });
    }
    out
}

/// Activate app `pid` the way a Dock click does, which also switches to its Space.
/// Needs no Accessibility permission. `Err` when no such app runs, `Ok(false)` when it has no
/// bundle to open.
pub fn open_app(pid: i32) -> Result<bool, &'static str> {
    let app =
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid).ok_or("App quit")?;
    Ok(open_running(&app))
}

pub(super) fn open_running(app: &NSRunningApplication) -> bool {
    let Some(url) = app.bundleURL() else { return false };
    NSWorkspace::sharedWorkspace().openURL(&url)
}

/// Hide the frontmost app, like cmd+H.
pub fn hide_frontmost() -> Result<(), &'static str> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication().ok_or("No frontmost app")?;
    app.hide();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_candidates_cover_paths_names_and_bundles() {
        let home = Path::new("/Users/me");
        assert!(app_candidates("  ", home).is_empty());
        assert_eq!(app_candidates("~/Apps/X.app", home), [PathBuf::from("/Users/me/Apps/X.app")]);
        assert_eq!(
            app_candidates("/Applications/X.app", home),
            [PathBuf::from("/Applications/X.app")]
        );
        let named = app_candidates(" Safari ", home);
        assert_eq!(named.first(), Some(&PathBuf::from("/Applications/Safari.app")));
        assert_eq!(named.last(), Some(&PathBuf::from("/Users/me/Applications/Safari.app")));
        assert_eq!(named.len(), APP_DIRS.len() + 1);
        assert_eq!(app_candidates("Safari.app", home), named);
    }
}
