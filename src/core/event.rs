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
    /// A config reload finished: every module gets it, after the modules the reload enabled
    /// got `Started`. A module that announces state to others (`CardsPending`) sends it
    /// again here, so a module that just started hears it (flick-b220).
    Reloaded,
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
    ModuleChanged { module: &'static str },
    /// Key chord `index` (into the `keys` module's chords) went down or up. Posted from the
    /// key tap thread with `events::post`.
    Chord { index: u16, down: bool },
    /// The focused window of followed app `pid` changed, or its title did (coalesced, at
    /// most once a second). Only while a module follows an app (`platform::axwatch`).
    WindowChanged { pid: i32 },
    /// The system is about to sleep.
    Sleep,
    /// The screen locked, or another user's session took over.
    Locked,
    /// The screen unlocked, or this session became active again.
    Unlocked,
    /// The running task changed: `task` is its id, `None` when no task runs. Posted by the
    /// `task` module with `events::post` on start, stop, switch and `Started`.
    TaskChanged { task: Option<i64> },
    /// The number of cards that wait on the user changed (or `Started`): `count` of them.
    /// Posted by the `message` module, which owns the cards, with `events::post`.
    CardsPending { count: u32 },
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
            (Event::Reloaded, r#"{"event":"reloaded"}"#),
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
            (Event::WindowChanged { pid: 1 }, r#"{"event":"window_changed","pid":1}"#),
            (Event::Sleep, r#"{"event":"sleep"}"#),
            (Event::Locked, r#"{"event":"locked"}"#),
            (Event::Unlocked, r#"{"event":"unlocked"}"#),
            (Event::TaskChanged { task: Some(3) }, r#"{"event":"task_changed","task":3}"#),
            (Event::TaskChanged { task: None }, r#"{"event":"task_changed","task":null}"#),
            (Event::CardsPending { count: 2 }, r#"{"event":"cards_pending","count":2}"#),
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
