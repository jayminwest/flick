//! Module `desktop`: the desktop toggle hotkey. There is no public Spaces API, so this goes
//! through apps: activate the most recently used app whose windows are all on another
//! Space, and macOS switches to that Space.

use std::collections::HashSet;

use crate::config::Config;
use crate::core::{Binding, Cx, Event, ListView, Module, RecentPids};
use crate::platform::{spaces, workspace};

/// `recent`: apps by activation, most recent first.
#[derive(Default)]
pub struct Desktop {
    recent: RecentPids,
}

impl Module for Desktop {
    fn id(&self) -> &'static str {
        "desktop"
    }

    /// `desktop_toggle` binds key `toggle`.
    fn hotkeys(&self, config: &Config) -> Vec<Binding> {
        config
            .desktop_toggle
            .iter()
            .map(|spec| Binding { spec: spec.clone(), key: Ok("toggle".into()) })
            .collect()
    }

    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        if key == "toggle"
            && let Err(e) = toggle(self.recent.pids())
        {
            eprintln!("flick: desktop toggle: {e}");
        }
        None
    }

    /// Track app activations, starting from the frontmost app.
    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        self.recent.observe(event, workspace::frontmost_pid);
        false
    }
}

/// Pick the app (other than `current`) with windows, none of them on this Space: the most recent
/// one in `recent`, else the first in `fallback` (history is empty right after Flick starts).
fn pick(
    recent: &[i32],
    fallback: &[i32],
    current: Option<i32>,
    here: &HashSet<i32>,
    anywhere: &HashSet<i32>,
) -> Option<i32> {
    recent
        .iter()
        .chain(fallback)
        .copied()
        .find(|pid| Some(*pid) != current && anywhere.contains(pid) && !here.contains(pid))
}

fn toggle(recent: &[i32]) -> Result<(), &'static str> {
    // Regular (Dock) apps only: menu-bar apps keep hidden windows but have no desktop to switch to.
    let regular = |pid: &i32| workspace::is_regular(*pid);
    let recent: Vec<i32> = recent.iter().copied().filter(regular).collect();
    let fallback: Vec<i32> = workspace::unhidden_app_pids().into_iter().filter(regular).collect();
    let current = workspace::frontmost_pid();
    let (here, anywhere) = (spaces::window_pids(true), spaces::window_pids(false));
    let pid =
        pick(&recent, &fallback, current, &here, &anywhere).ok_or("No app on another desktop")?;
    if !workspace::open_app(pid)? {
        return Err("App has no bundle");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_most_recent_app_on_another_space() {
        let here: HashSet<i32> = [1, 2].into();
        let anywhere: HashSet<i32> = [1, 2, 3, 4].into();
        // 2 is on this Space, 5 has no windows, so 3 wins over the older 4.
        assert_eq!(pick(&[1, 2, 5, 3, 4], &[], Some(1), &here, &anywhere), Some(3));
        assert_eq!(pick(&[1, 2], &[], Some(1), &here, &anywhere), None);
        // Empty history falls back to any app on another Space.
        assert_eq!(pick(&[1], &[2, 4], Some(1), &here, &anywhere), Some(4));
    }

    #[test]
    fn binds_desktop_toggle() {
        let config = Config { desktop_toggle: Some("cmd+Backquote".into()), ..Config::default() };
        let want = Binding { spec: "cmd+Backquote".into(), key: Ok("toggle".into()) };
        assert_eq!(Desktop::default().hotkeys(&config), [want]);
        assert!(Desktop::default().hotkeys(&Config::default()).is_empty());
    }
}
