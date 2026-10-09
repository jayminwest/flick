//! Installed application index, and the module that lists and opens apps.

use std::path::{Path, PathBuf};

use crate::core::{Cx, Event, Icon, Item, ItemId, Module, Outcome, unknown_verb};
use crate::platform::workspace;

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
}

impl Apps {
    pub fn new(apps: Vec<App>) -> Apps {
        Apps { apps }
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

    /// Rescans on `LauncherOpened`, so new apps show up. Root search re-ranks on every
    /// keystroke, so no view goes stale.
    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        if event == Event::LauncherOpened {
            self.apps = scan();
        }
        false
    }

    fn verbs(&self) -> &'static str {
        "app list | app open <name>"
    }

    /// `list`: `<name>\t<path>` per app. `open <name>`: open the app with that name, any case.
    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        match args {
            [verb] if verb == "list" => Ok(self
                .apps
                .iter()
                .map(|a| format!("{}\t{}", a.name, a.path.display()))
                .collect::<Vec<_>>()
                .join("\n")),
            [verb, name @ ..] if verb == "open" && !name.is_empty() => {
                let name = name.join(" ");
                let app = self.apps.iter().find(|a| a.name.eq_ignore_ascii_case(&name));
                let app = app.ok_or_else(|| format!("app: no app named \"{name}\""))?;
                workspace::open_file(&app.path);
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
}
