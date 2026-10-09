//! Module `desktop`: the desktop toggle hotkey. There is no public Spaces API, so this goes
//! through apps: activate the most recently used app whose windows are all on another
//! Space, and macOS switches to that Space.

use std::collections::HashSet;

use serde::Deserialize;

use crate::config::Section;
use crate::core::{Binding, Cx, Event, ListView, Module, RecentPids};
use crate::platform::{spaces, workspace};

/// Table `[desktop]`: `hotkey` (legacy: top-level `desktop_toggle`).
#[derive(Default, Deserialize)]
#[serde(default)]
struct Settings {
    hotkey: Option<String>,
}

/// `recent`: apps by activation, most recent first.
#[derive(Default)]
pub struct Desktop {
    hotkey: Option<String>,
    recent: RecentPids,
}

impl Module for Desktop {
    fn id(&self) -> &'static str {
        "desktop"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.hotkey = table.get::<Settings>()?.hotkey;
        Ok(())
    }

    /// `hotkey` binds key `toggle`.
    fn hotkeys(&self) -> Vec<Binding> {
        self.hotkey
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

    fn desktop(text: &str) -> Desktop {
        let config = crate::config::parse(text).unwrap();
        let mut desktop = Desktop::default();
        desktop.configure(&config.section("desktop").unwrap().unwrap()).unwrap();
        desktop
    }

    #[test]
    fn binds_desktop_toggle() {
        let want = Binding { spec: "cmd+Backquote".into(), key: Ok("toggle".into()) };
        assert_eq!(
            desktop("[desktop]\nhotkey = \"cmd+Backquote\"").hotkeys(),
            std::slice::from_ref(&want)
        );
        assert_eq!(desktop("desktop_toggle = \"cmd+Backquote\"").hotkeys(), [want]);
        assert!(desktop("").hotkeys().is_empty());
        let config = crate::config::parse("[desktop]\nhotkey = 1").unwrap();
        let section = config.section("desktop").unwrap().unwrap();
        assert!(Desktop::default().configure(&section).is_err());
    }

    #[test]
    fn the_example_config_lists_every_key() {
        crate::config::example::assert_documents::<Settings>("desktop");
    }
}
