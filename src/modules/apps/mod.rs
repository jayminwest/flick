//! Installed application index, and the module that lists, opens, reveals and quits apps.

mod actions;
mod leftovers;

use std::path::{Path, PathBuf};

use crate::core::{Action, Cx, Event, Icon, Item, ItemId, Module, Outcome, unknown_verb};
use crate::platform::{app, workspace};

#[derive(Clone)]
pub struct App {
    pub name: String,
    pub path: PathBuf,
}

const ROOTS: [&str; 4] = [
    "/Applications",
    "/System/Applications",
    "/System/Applications/Utilities",
    "/Applications/Utilities",
];

/// `command` verbs that take an app name.
const NAME_VERBS: [&str; 4] = ["open", "reveal", "quit", "force-quit"];

/// Scan the standard app folders, one level into subfolders (e.g. "/Applications/Adobe Photoshop/").
pub fn scan() -> Vec<App> {
    let mut apps = Vec::new();
    let home_apps = dirs::home_dir().map(|h| h.join("Applications"));
    let roots = ROOTS.iter().map(PathBuf::from).chain(home_apps);
    for root in roots {
        scan_dir(&root, 1, &mut apps);
    }
    apps.push(App {
        name: "Finder".into(),
        path: "/System/Library/CoreServices/Finder.app".into(),
    });
    apps.sort_by(|a, b| a.path.cmp(&b.path));
    apps.dedup_by(|a, b| a.name == b.name);
    apps
}

fn scan_dir(dir: &Path, depth: u32, out: &mut Vec<App>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "app") {
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                out.push(App { name: name.to_string(), path });
            }
        } else if depth > 0 && entry.file_type().is_ok_and(|t| t.is_dir()) {
            scan_dir(&path, depth - 1, out);
        }
    }
}

/// Module `app`: one root item per installed app; ids are `app:<bundle path>`.
pub struct Apps {
    apps: Vec<App>,
    /// Flick's own bundle, which offers no Quit.
    own: Option<PathBuf>,
}

impl Apps {
    pub fn new(apps: Vec<App>) -> Apps {
        Apps { apps, own: app::own_bundle() }
    }

    /// The indexed app named `name`, any case.
    fn named(&self, name: &str) -> Result<&App, String> {
        let app = self.apps.iter().find(|a| a.name.eq_ignore_ascii_case(name));
        app.ok_or_else(|| format!("app: no app named \"{name}\""))
    }
}

impl Module for Apps {
    fn id(&self) -> &'static str {
        "app"
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        self.apps
            .iter()
            .map(|a| Item {
                accessory: "Application".into(),
                ..Item::new(
                    ItemId::new("app", a.path.display()),
                    a.name.clone(),
                    "Open Application",
                    Icon::File(a.path.clone()),
                )
            })
            .collect()
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        cx.hide();
        workspace::open_file(Path::new(id.key()));
        Outcome::Hide
    }

    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        self.menu(Path::new(id.key()))
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        self.run_action(Path::new(id.key()), key, cx)
    }

    /// Rescans on `LauncherOpened` and `Wake`, so new apps show up, and on `Started` when it
    /// has no index yet (a config reload enabled it). Root search re-ranks on every
    /// keystroke, so no view goes stale.
    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        let rescan = match event {
            Event::LauncherOpened | Event::Wake => true,
            Event::Started => self.apps.is_empty(),
            _ => false,
        };
        if rescan {
            self.apps = scan();
        }
        false
    }

    fn verbs(&self) -> &'static str {
        "app list | app open|quit|force-quit|reveal <name>"
    }

    /// `list`: `<name>\t<path>` per app. `open|quit|force-quit|reveal <name>`: act on the app
    /// with that name, any case; `quit` prints the status, e.g. "Asked Safari to quit".
    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        match args {
            [verb] if verb == "list" => Ok(self
                .apps
                .iter()
                .map(|a| format!("{}\t{}", a.name, a.path.display()))
                .collect::<Vec<_>>()
                .join("\n")),
            [verb, name @ ..] if !name.is_empty() && NAME_VERBS.contains(&verb.as_str()) => {
                let path = self.named(&name.join(" "))?.path.clone();
                match verb.as_str() {
                    "open" => workspace::open_file(&path),
                    "reveal" => workspace::reveal(&path),
                    quit => {
                        let force = quit == "force-quit";
                        return self.quit(&path, force).map_err(|e| format!("app: {e}"));
                    }
                }
                Ok(String::new())
            }
            _ => Err(unknown_verb("app", args)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_cx;

    #[test]
    fn commands_list_apps_and_reject_unknown_ones() {
        let mut apps = Apps::new(vec![App { name: "Safari".into(), path: "/A/Safari.app".into() }]);
        test_cx("", |cx| {
            assert_eq!(apps.command(&["list".into()], cx).unwrap(), "Safari\t/A/Safari.app");
            let err = apps.command(&["open".into(), "No".into(), "Such".into()], cx).unwrap_err();
            assert_eq!(err, "app: no app named \"No Such\"");
            assert_eq!(
                apps.command(&["open".into()], cx).unwrap_err(),
                "app: unknown command \"open\""
            );
        });
    }

    #[test]
    fn started_scans_only_an_empty_index() {
        let safari = App { name: "Safari".into(), path: "/A/Safari.app".into() };
        let names = |a: &Apps| a.apps.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
        let mut kept = Apps::new(vec![safari]);
        let mut fresh = Apps::new(vec![]);
        test_cx("", |cx| {
            assert!(!kept.on_event(Event::Started, cx));
            assert!(!fresh.on_event(Event::Started, cx));
        });
        assert_eq!(names(&kept), ["Safari"]);
        // A scan always finds Finder.
        assert!(names(&fresh).contains(&"Finder".to_string()));
    }

    #[test]
    fn wake_rescans_and_other_events_do_not() {
        let gone = App { name: "Gone".into(), path: "/nowhere/Gone.app".into() };
        let mut apps = Apps::new(vec![gone]);
        let has_gone = |a: &Apps| a.apps.iter().any(|a| a.name == "Gone");
        test_cx("", |cx| {
            for event in [Event::DisplaysChanged, Event::PasteboardChanged, Event::Active] {
                assert!(!apps.on_event(event, cx));
            }
            assert!(has_gone(&apps));
            assert!(!apps.on_event(Event::Wake, cx));
        });
        assert!(!has_gone(&apps));
    }
}
