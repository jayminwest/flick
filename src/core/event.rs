//! Events: things that happen outside any module, dispatched to every module in order on
//! the main thread. `Serialize` gives each one a stable JSON shape, `{"event":"<name>",...}`
//! with `snake_case` names, for streams such as `flick events`.

use serde::Serialize;

/// Something that happened outside any module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The controller started, before the run loop.
    Started,
    /// The launcher opened at root search.
    LauncherOpened,
    /// App `pid` became frontmost.
    AppActivated { pid: i32 },
    /// Some app wrote the pasteboard.
    PasteboardChanged,
    /// The system woke from sleep.
    Wake,
    /// A display was added, removed, moved or resized.
    DisplaysChanged,
    /// No keyboard or mouse input for `secs` seconds. Sent once per idle stretch.
    Idle { secs: u64 },
    /// Input resumed after `Idle`.
    Active,
    /// Background work of module `module` progressed: its visible view refreshes, whatever
    /// its `on_event` returns. Posted from the module's own thread with `events::post`.
    #[cfg_attr(not(test), expect(dead_code, reason = "posted by the rebuild runner, flick-31c8"))]
    ModuleChanged { module: &'static str },
    /// Key chord `index` (into the `keys` module's chords) went down or up. Posted from the
    /// key tap thread with `events::post`.
    Chord { index: u16, down: bool },
}

/// Pids of activated apps, most recent first, at most `RECENT_MAX`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentPids(Vec<i32>);

const RECENT_MAX: usize = 32;

impl RecentPids {
    /// Move `pid` to the front.
    pub fn record(&mut self, pid: i32) {
        self.0.retain(|&p| p != pid);
        self.0.insert(0, pid);
        self.0.truncate(RECENT_MAX);
    }

    /// Record the app `event` activates; `Started` records `frontmost()`.
    pub fn observe(&mut self, event: Event, frontmost: fn() -> Option<i32>) {
        let pid = match event {
            Event::Started => frontmost(),
            Event::AppActivated { pid } => Some(pid),
            _ => None,
        };
        if let Some(pid) = pid {
            self.record(pid);
        }
    }

    pub fn pids(&self) -> &[i32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_to_stable_names() {
        let cases = [
            (Event::Started, r#"{"event":"started"}"#),
            (Event::LauncherOpened, r#"{"event":"launcher_opened"}"#),
            (Event::AppActivated { pid: 42 }, r#"{"event":"app_activated","pid":42}"#),
            (Event::PasteboardChanged, r#"{"event":"pasteboard_changed"}"#),
            (Event::Wake, r#"{"event":"wake"}"#),
            (Event::DisplaysChanged, r#"{"event":"displays_changed"}"#),
            (Event::Idle { secs: 60 }, r#"{"event":"idle","secs":60}"#),
            (Event::Active, r#"{"event":"active"}"#),
            (
                Event::ModuleChanged { module: "flick" },
                r#"{"event":"module_changed","module":"flick"}"#,
            ),
            (Event::Chord { index: 0, down: true }, r#"{"event":"chord","index":0,"down":true}"#),
        ];
        for (event, json) in cases {
            assert_eq!(serde_json::to_string(&event).unwrap(), json);
        }
    }

    #[test]
    fn recent_pids_are_most_recent_first_deduped_and_capped() {
        let mut recent = RecentPids::default();
        for pid in [1, 2, 3, 2] {
            recent.record(pid);
        }
        assert_eq!(recent.pids(), [2, 3, 1]);
        for pid in 100..140 {
            recent.record(pid);
        }
        assert_eq!(recent.pids().len(), RECENT_MAX);
        assert_eq!(recent.pids()[0], 139);
    }

    #[test]
    fn recent_pids_follow_activations_from_the_frontmost_app() {
        let mut recent = RecentPids::default();
        recent.observe(Event::Started, || Some(7));
        recent.observe(Event::AppActivated { pid: 8 }, || None);
        recent.observe(Event::Wake, || Some(9));
        recent.observe(Event::Started, || None);
        assert_eq!(recent.pids(), [8, 7]);
    }
}
