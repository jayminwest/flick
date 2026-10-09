//! Window switcher order: apps by most recent activation, the frontmost app's window last
//! (so the first entry is the previous window), and apps whose windows are all on another
//! desktop collapsed into one entry.

use std::collections::HashSet;

use crate::modules::switcher::list::{AppWindow, AppWithWindows, arrange};
use crate::platform::workspace::RunningApp;

fn app(pid: i32, name: &str, hidden: bool, windows: &[(&str, bool)]) -> AppWithWindows<u32> {
    let running = RunningApp {
        pid,
        name: name.into(),
        bundle: Some(format!("/Applications/{name}.app").into()),
        hidden,
    };
    let windows = windows
        .iter()
        .enumerate()
        .map(|(i, &(title, minimized))| {
            ((!title.is_empty()).then(|| title.to_string()), minimized, pid as u32 * 10 + i as u32)
        })
        .collect();
    (running, windows)
}

/// `pid title app [min] [other]` per entry.
fn show(list: &[AppWindow<u32>]) -> Vec<String> {
    list.iter()
        .map(|w| {
            let mut s = format!("{} {} {}", w.pid, w.title, w.app);
            if w.minimized {
                s += " min";
            }
            if w.on_other_desktop() {
                s += " other";
            }
            s
        })
        .collect()
}

fn apps() -> Vec<AppWithWindows<u32>> {
    vec![
        app(1, "Finder", false, &[("Home", false)]),
        app(2, "Safari", false, &[("Docs", false), ("", true)]),
        app(3, "Mail", false, &[]),
        app(4, "Notes", true, &[]),
        app(5, "Music", false, &[]),
        app(6, "Slack", false, &[("General", false)]),
    ]
}

#[test]
fn recent_apps_first_frontmost_last_other_desktop_collapsed() {
    // Mail and Notes have windows on another desktop; Notes is hidden, Music has none.
    let elsewhere: HashSet<i32> = [3, 4].into();
    let out = arrange(apps(), &elsewhere, &[2, 6, 3], Some(2));
    assert_eq!(
        show(&out),
        [
            "2 Safari Safari min",
            "6 General Slack",
            "3 Mail Mail other",
            "1 Home Finder",
            "2 Docs Safari",
        ]
    );
}

#[test]
fn no_history_keeps_system_order() {
    let out = arrange(apps(), &HashSet::new(), &[], None);
    assert_eq!(
        show(&out),
        ["1 Home Finder", "2 Docs Safari", "2 Safari Safari min", "6 General Slack"]
    );
}

#[test]
fn frontmost_without_windows_changes_nothing() {
    let out = arrange(apps(), &HashSet::new(), &[6], Some(5));
    assert_eq!(
        show(&out),
        ["6 General Slack", "1 Home Finder", "2 Docs Safari", "2 Safari Safari min"]
    );
}
