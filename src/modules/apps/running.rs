//! The Quit Applications view: running regular apps, where Enter asks one to quit.
//!
//! Root item `app:quit` opens view `app/running` (safe: app keys are absolute paths). Its items
//! are `app:<bundle path>` with arg `quit`, so cmd+K offers the same actions as the app's root
//! item, and Enter runs the module's Quit (`Apps::quit`, which never quits Flick). Flick itself
//! is never listed.

use std::path::Path;

use crate::core::{Cx, Icon, Item, ItemId, ListView};
use crate::platform::workspace::{self, RunningApp};

/// Key of the root item that opens the view.
pub const ROOT_KEY: &str = "quit";
/// The view's name, and the arg its items carry.
pub const VIEW: &str = "running";
pub const QUIT_ARG: &str = "quit";

/// The root item "Quit Applications".
pub fn root_item() -> Item {
    Item {
        accessory: "Command".into(),
        keywords: vec!["running apps close terminate".into()],
        ..Item::new(
            ItemId::new("app", ROOT_KEY),
            "Quit Applications",
            "Open Command",
            Icon::Symbol("xmark.circle"),
        )
    }
}

/// View `running`, empty until `refresh` fills it.
pub fn view() -> ListView {
    ListView {
        placeholder: "Search running apps…".into(),
        footer: "Enter quits the selected app".into(),
        empty: "No running apps".into(),
        ..ListView::new("app", VIEW)
    }
}

/// Running regular apps other than Flick (this process, or another one from bundle `own`), in
/// the system's order.
pub fn running(own: Option<&Path>) -> Vec<RunningApp> {
    let me = i32::try_from(std::process::id()).unwrap_or(-1);
    without_flick(workspace::regular_apps(me), own)
}

/// `apps` without Flick's own bundle (another Flick process from the same bundle included).
fn without_flick(apps: Vec<RunningApp>, own: Option<&Path>) -> Vec<RunningApp> {
    apps.into_iter().filter(|a| own.is_none() || a.bundle.as_deref() != own).collect()
}

/// One item per app with a bundle (an app without one has no `app:<path>` id), ranked for
/// `cx.query` in the system's order for equal scores.
pub fn refresh(view: &mut ListView, apps: &[RunningApp], cx: &mut Cx) {
    let items = apps
        .iter()
        .filter_map(|a| {
            let path = a.bundle.as_ref()?;
            Some(Item {
                accessory: if a.hidden { "Hidden".into() } else { String::new() },
                ..Item::new(
                    ItemId::new("app", path.display()).with_arg(QUIT_ARG),
                    a.name.clone(),
                    "Quit Application",
                    Icon::File(path.clone()),
                )
            })
        })
        .collect::<Vec<_>>();
    let order: Vec<String> = items.iter().map(|i| i.id.to_string()).collect();
    view.items = cx.ranker.rank(cx.query, items, |item| {
        let i = order.iter().position(|id| id == item.id.as_str()).unwrap_or(0);
        -(i as f64) * 1e-3
    });
}

/// `app running`: `<pid>\t<name>\t<path>` per app; the path is empty for an app without a
/// bundle.
pub fn lines(apps: &[RunningApp]) -> String {
    apps.iter()
        .map(|a| {
            let path = a.bundle.as_deref().map(Path::display);
            format!("{}\t{}\t{}", a.pid, a.name, path.map(|p| p.to_string()).unwrap_or_default())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::core::test_cx;

    fn app(pid: i32, name: &str, bundle: Option<&str>, hidden: bool) -> RunningApp {
        RunningApp { pid, name: name.into(), bundle: bundle.map(PathBuf::from), hidden }
    }

    fn apps() -> Vec<RunningApp> {
        vec![
            app(10, "Safari", Some("/Applications/Safari.app"), false),
            app(11, "Helper", None, false),
            app(12, "Slack", Some("/Applications/Slack.app"), true),
        ]
    }

    #[test]
    fn view_lists_bundled_apps_in_system_order_with_quit_ids() {
        let mut view = view();
        assert!(view.is("app", "running") && !view.record_use);
        test_cx("", |cx| refresh(&mut view, &apps(), cx));
        let ids: Vec<_> = view.items.iter().map(|i| (i.id.as_str(), i.id.arg())).collect();
        assert_eq!(
            ids,
            [
                ("app:/Applications/Safari.app", Some("quit")),
                ("app:/Applications/Slack.app", Some("quit")),
            ]
        );
        assert_eq!(view.items[0].verb, "Quit Application");
        assert_eq!(view.items[1].accessory, "Hidden");
        assert_eq!(view.items[0].icon, Icon::File("/Applications/Safari.app".into()));

        test_cx("sla", |cx| refresh(&mut view, &apps(), cx));
        assert_eq!(view.items.len(), 1);
        assert_eq!(view.items[0].title, "Slack");
    }

    #[test]
    fn root_item_is_keyed_quit_which_no_app_path_can_be() {
        let item = root_item();
        assert_eq!(item.id.as_str(), "app:quit");
        assert_eq!(item.title, "Quit Applications");
        assert!(!item.id.key().starts_with('/'));
    }

    #[test]
    fn flick_is_never_listed() {
        let own = Path::new("/Applications/Flick.app");
        let mut all = apps();
        all.push(app(13, "Flick", Some("/Applications/Flick.app"), false));
        let names = |v: Vec<RunningApp>| v.into_iter().map(|a| a.name).collect::<Vec<_>>();
        assert_eq!(names(without_flick(all, Some(own))), ["Safari", "Helper", "Slack"]);
        assert_eq!(names(without_flick(apps(), None)).len(), 3);
    }

    #[test]
    fn lines_print_pid_name_and_path() {
        assert_eq!(
            lines(&apps()),
            "10\tSafari\t/Applications/Safari.app\n11\tHelper\t\n12\tSlack\t/Applications/Slack.app"
        );
        assert_eq!(lines(&[]), "");
    }
}
