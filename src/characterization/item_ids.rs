//! Item ids are written to the `usage` table; a changed id loses that item's history.

use super::legacy_view::Config;
use crate::core::{Item, ListView, test_cx};
use crate::modules::apps::App;
use crate::modules::quicklinks::Quicklink;
use crate::modules::with_apps;
use crate::root::root_items;

fn app(name: &str, path: &str) -> App {
    App { name: name.into(), path: path.into() }
}

fn ids(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.id.as_str()).collect()
}

#[test]
fn app_ids_are_the_bundle_path() {
    let apps = [
        app("Safari", "/Applications/Safari.app"),
        app("Adobe Photoshop 2025", "/Applications/Adobe Photoshop 2025/Adobe Photoshop 2025.app"),
        app("Finder", "/System/Library/CoreServices/Finder.app"),
    ];
    let items = root_items(&apps, &[]);
    assert_eq!(
        ids(&items[..3]),
        [
            "app:/Applications/Safari.app",
            "app:/Applications/Adobe Photoshop 2025/Adobe Photoshop 2025.app",
            "app:/System/Library/CoreServices/Finder.app",
        ]
    );
    assert_eq!(items[0].title, "Safari");
}

#[test]
fn window_ids_are_the_command_title() {
    let items = root_items(&[], &[]);
    let windows: Vec<&str> =
        ids(&items).into_iter().filter(|id| id.starts_with("window:")).collect();
    assert_eq!(
        windows,
        [
            "window:Left Half",
            "window:Right Half",
            "window:Top Half",
            "window:Bottom Half",
            "window:Top Left Quarter",
            "window:Top Right Quarter",
            "window:Bottom Left Quarter",
            "window:Bottom Right Quarter",
            "window:First Third",
            "window:Center Third",
            "window:Last Third",
            "window:First Two Thirds",
            "window:Last Two Thirds",
            "window:Maximize",
            "window:Almost Maximize",
            "window:Center",
            "window:Next Display",
            "window:Previous Display",
            "window:Minimize",
            "window:Hide",
        ]
    );
}

#[test]
fn quicklink_ids_are_the_name() {
    let links = [
        Quicklink {
            name: "Google".into(),
            url: "https://g.co/?q={query}".into(),
            keyword: None,
            app: None,
        },
        Quicklink {
            name: "My Repo".into(),
            url: "~/Projects".into(),
            keyword: Some("r".into()),
            app: None,
        },
    ];
    let items = root_items(&[], &links);
    let quick: Vec<&str> =
        ids(&items).into_iter().filter(|id| id.starts_with("quicklink:")).collect();
    assert_eq!(quick, ["quicklink:Google", "quicklink:My Repo"]);
    let defaults = root_items(&[], &Config::default().quicklinks);
    let quick: Vec<&str> =
        ids(&defaults).into_iter().filter(|id| id.starts_with("quicklink:")).collect();
    assert_eq!(
        quick,
        ["quicklink:Google", "quicklink:GitHub Search", "quicklink:YouTube", "quicklink:Projects"]
    );
}

#[test]
fn builtin_ids_are_the_title() {
    let items = root_items(&[], &[]);
    let builtins: Vec<&str> =
        ids(&items).into_iter().filter(|id| id.starts_with("builtin:")).collect();
    assert_eq!(
        builtins,
        [
            "builtin:Clipboard History",
            "builtin:Switch Windows",
            "builtin:Create Quicklink",
            "builtin:Open Flick Config",
            "builtin:Reload Flick Config",
            "builtin:Quit Flick",
        ]
    );
}

#[test]
fn root_items_are_apps_then_windows_then_quicklinks_then_builtins() {
    let apps = [app("Zed", "/Applications/Zed.app")];
    let links = [Quicklink {
        name: "Docs".into(),
        url: "https://docs.rs".into(),
        keyword: None,
        app: None,
    }];
    let items = root_items(&apps, &links);
    // Only these modules' items; a new module's root items are its own to pin.
    let known = ["app", "window", "quicklink", "builtin"];
    let prefixes: Vec<&str> =
        items.iter().filter_map(|i| i.id.split(':').next()).filter(|p| known.contains(p)).collect();
    // Apps: one per app plus "Quit Applications" (app:quit, flick-91a6).
    assert_eq!(prefixes.len(), 2 + 20 + 1 + 6);
    let mut runs = prefixes.clone();
    runs.dedup();
    assert_eq!(runs, known);
}

#[test]
fn capture_ids_are_fixed_keys() {
    let items = root_items(&[], &[]);
    let capture: Vec<&str> =
        ids(&items).into_iter().filter(|id| id.starts_with("capture:")).collect();
    assert_eq!(capture, ["capture:area", "capture:window", "capture:screen", "capture:recent"]);
    // Recent Captures rows are `capture:shot/<row id>`.
    let file = std::env::temp_dir().join(format!("flick-char-{}-shot.png", std::process::id()));
    std::fs::write(&file, b"png").unwrap();
    let mut registry = with_apps(&crate::config::Config::default(), vec![]).unwrap();
    let shots = test_cx("", |cx| {
        registry.migrate(cx.store).unwrap();
        cx.store
            .conn()
            .execute(
                "INSERT INTO capture_shots (id, path, kind, width, height, taken) VALUES (7, ?1, 'area', 1, 1, 0)",
                [file.display().to_string()],
            )
            .unwrap();
        let mut view = registry.open(&ListView::new("capture", "recent"), cx).unwrap();
        registry.refresh(&mut view, cx);
        view.items.into_iter().map(|i| i.id.to_string()).collect::<Vec<_>>()
    });
    std::fs::remove_file(file).unwrap();
    assert_eq!(shots, ["capture:shot/7"]);
}

#[test]
fn task_ids_are_fixed_keys() {
    // While no task runs; a running task swaps `task:start` for `task:stop` and `task:switch`.
    // Rows in `task/pick` and `task/today` are `task:run/<id>`, `task:new` and
    // `task:project/<name>`; those views do not record use.
    let items = root_items(&[], &[]);
    let task: Vec<&str> = ids(&items).into_iter().filter(|id| id.starts_with("task:")).collect();
    assert_eq!(task, ["task:start", "task:today"]);
}
