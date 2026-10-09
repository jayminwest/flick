//! The Quit Applications view: running regular apps, where Enter asks one to quit.
//!
//! Root item `app:quit` opens view `app/running` (safe: app keys are absolute paths). Its items
//! are `app:<bundle path>` with arg `quit`, so cmd+K offers the same actions as the app's root
//! item. Flick itself is never listed and never asked to quit.

use std::path::Path;

use crate::core::{Cx, Icon, Item, ItemId, ListView, Outcome};
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

/// Running regular apps other than Flick, in the system's order.
pub fn running() -> Vec<RunningApp> {
    let own = crate::platform::app::own_bundle();
    without_flick(workspace::regular_apps(own_pid()), own.as_deref())
}

fn own_pid() -> i32 {
    i32::try_from(std::process::id()).unwrap_or(-1)
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

/// Ask every running copy of the app at `path` to quit, never Flick itself. `terminate` sends
/// the request (`workspace::terminate`; tests pass a fake). The status names the app.
pub fn quit(path: &Path, pids: &[i32], terminate: impl Fn(i32) -> bool) -> Outcome {
    let name = path.file_stem().map_or_else(|| path.display().to_string(), |s| s.to_string_lossy().into());
    let own = crate::platform::app::own_bundle();
    if own.as_deref() == Some(path) {
        return Outcome::Stay(Some("Flick cannot quit itself here".into()));
    }
    let me = own_pid();
    let pids: Vec<i32> = pids.iter().copied().filter(|&p| p != me).collect();
    if pids.is_empty() {
        return Outcome::Stay(Some(format!("{name} is not running")));
    }
    let sent = pids.iter().filter(|&&p| terminate(p)).count();
    Outcome::Stay(Some(if sent == 0 {
        format!("{name} did not accept the quit request")
    } else {
        format!("Asked {name} to quit")
    }))
}

/// `quit` for the running copies of `path`, with the real terminate.
pub fn quit_running(path: &Path) -> Outcome {
    quit(path, &workspace::running_for_bundle(path), workspace::terminate)
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
    use std::cell::RefCell;
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

    fn status(outcome: Outcome) -> String {
        match outcome {
            Outcome::Stay(Some(s)) => s,
            other => panic!("expected a status, got {other:?}"),
        }
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
    fn quit_asks_each_pid_and_reports_the_result() {
        let asked = RefCell::new(vec![]);
        let accept = |pid| {
            asked.borrow_mut().push(pid);
            true
        };
        let path = Path::new("/Applications/Safari.app");
        assert_eq!(status(quit(path, &[10, 20], accept)), "Asked Safari to quit");
        assert_eq!(*asked.borrow(), [10, 20]);
        assert_eq!(status(quit(path, &[], |_| true)), "Safari is not running");
        assert_eq!(
            status(quit(path, &[10], |_| false)),
            "Safari did not accept the quit request"
        );
    }

    #[test]
    fn quit_never_asks_flick_itself() {
        let asked = RefCell::new(vec![]);
        let me = own_pid();
        let path = Path::new("/Applications/Safari.app");
        let out = quit(path, &[me], |pid| {
            asked.borrow_mut().push(pid);
            true
        });
        assert_eq!(status(out), "Safari is not running");
        assert!(asked.borrow().is_empty());
    }

    #[test]
    fn quit_running_leaves_a_spawned_child_alone() {
        // A plain child process is not an app bundle's process: nothing is asked to quit and it
        // keeps running. No real app is ever terminated.
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let out = quit_running(Path::new("/nonexistent/flick/Nope.app"));
        assert_eq!(status(out), "Nope is not running");
        assert!(child.try_wait().unwrap().is_none(), "child must still run");
        child.kill().unwrap();
        child.wait().unwrap();
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
