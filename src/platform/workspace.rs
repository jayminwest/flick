//! Running apps, opening files and URLs (`NSWorkspace`). Activation events come from
//! `super::events`.

use std::path::{Path, PathBuf};

use objc2_app_kit::{
    NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{NSArray, NSBundle, NSString, NSURL};

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

/// Some app with bundle identifier `id` ("com.knollsoft.Hyperkey") is running.
pub fn is_running(id: &str) -> bool {
    !NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(id))
        .is_empty()
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

/// The bundle identifier of the app bundle at `path` ("com.apple.Safari"); `None` when it is
/// not a bundle or declares none. Reads its Info.plist, so call it per action, not per scan.
#[cfg_attr(not(test), expect(dead_code, reason = "first caller is the uninstall (flick-4077)"))]
pub fn bundle_id(path: &Path) -> Option<String> {
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&path.display().to_string()))?;
    bundle.bundleIdentifier().map(|id| id.to_string()).filter(|id| !id.is_empty())
}

/// The bundle identifier ("com.apple.Safari") and display name ("Safari") of running app
/// `pid`. `None` when no app runs with that pid; the id is `None` for an app without one.
/// The by-pid sibling of `bundle_id`: cheap enough to call on every activation.
pub fn app_identity(pid: i32) -> Option<(Option<String>, String)> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    let id = app.bundleIdentifier().map(|id| id.to_string()).filter(|id| !id.is_empty());
    let name = app.localizedName().map(|n| n.to_string()).unwrap_or_default();
    Some((id, name))
}

/// Pids of the running apps launched from the bundle at `path`, in the system's order.
/// Matches the bundle's location, so a second copy of the same app is not included.
pub fn running_for_bundle(path: &Path) -> Vec<i32> {
    let want = canonical(path);
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|app| {
            let bundle = app.bundleURL().and_then(|u| u.path());
            bundle.is_some_and(|p| canonical(Path::new(&p.to_string())) == want)
        })
        .map(|app| app.processIdentifier())
        .collect()
}

/// `path` with symlinks resolved, or as given when it cannot be resolved (e.g. it is gone).
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Ask app `pid` to quit, like cmd+Q: it may ask to save or refuse. Asynchronous; `true` only
/// means the request was sent. `false` when `pid` is not a running app.
pub fn terminate(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .is_some_and(|a| a.terminate())
}

/// Force app `pid` to quit, like Force Quit: unsaved work is lost. `false` when `pid` is not a
/// running app or the request failed.
pub fn force_terminate(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .is_some_and(|a| a.forceTerminate())
}

/// Show the file, folder or bundle at `path` selected in a Finder window. Untested: it opens
/// Finder.
pub fn reveal(path: &Path) {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.display().to_string()));
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
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

    /// A minimal `.app` bundle with id `bid` in a fresh temp folder.
    fn fake_bundle(name: &str, bid: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flk-{}-ws-{name}", std::process::id()));
        let app = dir.join(format!("{name}.app"));
        let contents = app.join("Contents");
        std::fs::create_dir_all(&contents).unwrap();
        let key = bid.map_or(String::new(), |b| {
            format!("<key>CFBundleIdentifier</key><string>{b}</string>")
        });
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>CFBundleName</key><string>{name}</string>{key}</dict></plist>"
        );
        std::fs::write(contents.join("Info.plist"), plist).unwrap();
        app
    }

    #[test]
    fn bundle_id_reads_the_info_plist() {
        assert_eq!(
            bundle_id(&fake_bundle("WithId", Some("dev.flick.ws-test"))).as_deref(),
            Some("dev.flick.ws-test")
        );
        assert_eq!(bundle_id(&fake_bundle("NoId", None)), None);
        assert_eq!(bundle_id(Path::new("/nonexistent/flick/Nope.app")), None);
    }

    #[test]
    fn an_app_that_is_not_running_has_no_pids() {
        assert!(running_for_bundle(&fake_bundle("Idle", Some("dev.flick.ws-idle"))).is_empty());
        assert!(running_for_bundle(Path::new("/nonexistent/flick/Nope.app")).is_empty());
    }

    #[test]
    fn app_identity_names_running_apps_only() {
        let out = std::process::Command::new("/usr/bin/pgrep").args(["-x", "Dock"]).output();
        let dock = out.ok().and_then(|o| String::from_utf8(o.stdout).ok());
        if let Some(pid) = dock.and_then(|p| p.lines().next()?.trim().parse::<i32>().ok()) {
            let (id, name) = app_identity(pid).unwrap();
            assert_eq!(id.as_deref(), Some("com.apple.dock"));
            assert_eq!(name, "Dock");
        }
        assert!(app_identity(-1).is_none());
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        assert!(app_identity(i32::try_from(child.id()).unwrap()).is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn terminate_ignores_pids_that_are_not_apps() {
        // A plain child process is not an NSRunningApplication: both calls refuse it and it
        // keeps running. No real app is ever asked to quit.
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        assert!(!terminate(pid));
        assert!(!force_terminate(pid));
        assert!(child.try_wait().unwrap().is_none(), "child must still run");
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!terminate(-1));
        assert!(!force_terminate(-1));
    }

    #[test]
    fn canonical_resolves_symlinks_and_keeps_missing_paths() {
        let missing = Path::new("/nonexistent/flick/X.app");
        assert_eq!(canonical(missing), missing);
        let app = fake_bundle("Linked", Some("dev.flick.ws-linked"));
        let link = app.with_file_name("Link.app");
        std::os::unix::fs::symlink(&app, &link).unwrap();
        assert_eq!(canonical(&link), canonical(&app));
    }
}
