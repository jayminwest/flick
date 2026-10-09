//! The windows the switcher lists, and focusing one.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::platform::{ax, spaces, workspace};

/// A window to switch to. `element` is None for an app whose windows are on another
/// desktop: Accessibility only lists windows on the current one.
pub struct AppWindow<E = ax::AxWindow> {
    pub pid: i32,
    pub title: String,
    pub app: String,
    pub bundle: Option<PathBuf>,
    pub minimized: bool,
    element: Option<E>,
}

impl<E> AppWindow<E> {
    pub fn on_other_desktop(&self) -> bool {
        self.element.is_none()
    }
}

/// A running app with its standard windows on this desktop, front to back, as
/// `(title, minimized, element)`.
pub type AppWithWindows<E> = (workspace::RunningApp, Vec<(Option<String>, bool, E)>);

/// Standard windows of every regular app, apps in `recent` order (most recent first).
/// The frontmost window goes last, so the first entry is the previous window.
pub fn list_windows(recent: &[i32], frontmost: Option<i32>) -> Vec<AppWindow> {
    if !ax::ensure_trusted() {
        return vec![];
    }
    let me = std::process::id() as i32;
    let elsewhere = spaces::window_pids(false);
    let apps = workspace::regular_apps(me)
        .into_iter()
        .map(|app| {
            let windows = ax::standard_windows(app.pid)
                .into_iter()
                .map(|w| (w.title.clone(), w.minimized, w))
                .collect();
            (app, windows)
        })
        .collect();
    arrange(apps, &elsewhere, recent, frontmost)
}

/// The switcher's list: each app's windows (titled by the app when untitled), or one entry
/// for an unhidden app with no window here but windows in `elsewhere` (another desktop).
/// Apps sort by `recent` (unlisted last, else system order); the `frontmost` app's first
/// window goes last.
pub fn arrange<E>(
    apps: Vec<AppWithWindows<E>>,
    elsewhere: &HashSet<i32>,
    recent: &[i32],
    frontmost: Option<i32>,
) -> Vec<AppWindow<E>> {
    let mut out = Vec::new();
    for (app, windows) in apps {
        let (pid, name, bundle) = (app.pid, app.name, app.bundle);
        let before = out.len();
        for (title, minimized, element) in windows {
            let title = title.filter(|t| !t.is_empty()).unwrap_or_else(|| name.clone());
            out.push(AppWindow {
                pid,
                title,
                app: name.clone(),
                bundle: bundle.clone(),
                minimized,
                element: Some(element),
            });
        }
        if out.len() == before && elsewhere.contains(&pid) && !app.hidden {
            out.push(AppWindow {
                pid,
                title: name.clone(),
                app: name,
                bundle,
                minimized: false,
                element: None,
            });
        }
    }

    let rank = |pid: i32| recent.iter().position(|&p| p == pid).unwrap_or(usize::MAX);
    out.sort_by_key(|w| rank(w.pid)); // stable: keeps each app's front-to-back order
    if let Some(i) = out.iter().position(|w| Some(w.pid) == frontmost) {
        let current = out.remove(i);
        out.push(current);
    }
    out
}

/// Raise `w` and activate its app (switching desktops if needed).
pub fn focus(w: &AppWindow) {
    ax::focus(w.pid, w.element.as_ref(), w.minimized);
}
