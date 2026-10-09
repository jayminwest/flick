//! `flick activity status`: recording, the title and URL switches with their permissions,
//! and the open span.

use crate::core::store::Store;

use super::Activity;
use super::report::local_time;
use super::store::Spans;

impl Activity {
    pub(super) fn status(&self, store: &Store) -> String {
        let on = if store.recording() { "on" } else { "off" };
        let titles = match (self.config.titles, (self.env.trusted)()) {
            (false, _) => "off",
            (true, true) => "on",
            (true, false) => "no Accessibility permission",
        };
        let urls = match (self.config.urls, self.urls.denied()) {
            (false, _) => "off".into(),
            (true, []) => "on".into(),
            (true, names) => format!("no Automation permission for {}", names.join(", ")),
        };
        let open = match self.clock.open() {
            Some((s, start)) => {
                let since = local_time(start, (self.env.utc_offset)(start));
                format!("{} since {}", s.name, &since[11..])
            }
            None if self.away.any() => "screen locked or asleep".into(),
            None if self.excluded && self.clock.is_paused() => "excluded app in front".into(),
            None if self.clock.is_idle() => "idle".into(),
            None => "none".into(),
        };
        format!("recording: {on}\ntitles: {titles}\nurls: {urls}\nopen span: {open}")
    }
}
