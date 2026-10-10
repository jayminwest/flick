//! The action menu (cmd+K) on app items, and the `app quit | force-quit | reveal` verbs.
//! Uninstall… is in `uninstall.rs`.

use std::path::Path;

use super::Apps;
use crate::core::{Action, Cx, Icon, Outcome};
use crate::platform::workspace;

const OPEN: &str = "open";
const REVEAL: &str = "reveal";
const QUIT: &str = "quit";
const FORCE_QUIT: &str = "force-quit";
const UNINSTALL: &str = "uninstall";

impl Apps {
    /// cmd+K on the app at `path`. The panel asks on every render of the selected item, so
    /// this reads the cached running apps (`Apps::running`, one `AppKit` query after each
    /// launch, quit or launcher open): Quit and Force Quit are listed for running apps but
    /// Flick itself, and `quit` checks again. Uninstall… is left out for protected apps (see
    /// `uninstallable`). Empty for a key that is not a path.
    pub(super) fn menu(&mut self, path: &Path) -> Vec<Action> {
        if !path.is_absolute() {
            return vec![];
        }
        let mut menu = vec![
            Action::new(OPEN, "Open Application", Icon::Symbol("arrow.up.forward.app")),
            Action::new(REVEAL, "Show in Finder", Icon::Symbol("folder")),
        ];
        if !self.is_own(path) && self.is_running(path) {
            menu.push(Action::new(QUIT, "Quit", Icon::Symbol("xmark.circle")));
            menu.push(Action::new(
                FORCE_QUIT,
                "Force Quit",
                Icon::Symbol("exclamationmark.octagon"),
            ));
        }
        if self.uninstallable(path) {
            menu.push(Action::new(UNINSTALL, "Uninstall…", Icon::Symbol("trash")));
        }
        menu
    }

    /// Run menu action `key` on the app at `path`.
    pub(super) fn run_action(&mut self, path: &Path, key: &str, cx: &mut Cx) -> Outcome {
        match key {
            OPEN => {
                cx.hide();
                workspace::open_file(path);
                Outcome::Hide
            }
            REVEAL => {
                cx.hide();
                workspace::reveal(path);
                Outcome::Hide
            }
            QUIT | FORCE_QUIT => {
                let status = self.quit(path, key == FORCE_QUIT);
                Outcome::Stay(Some(status.unwrap_or_else(|e| e)))
            }
            UNINSTALL => self.ask_uninstall(path),
            _ => Outcome::Stay(None),
        }
    }

    /// Ask every running copy of the app at `path` to quit (`force`: force it). `Ok` is a
    /// status; quitting is asynchronous and the app may refuse, so it says "Asked". Never
    /// quits Flick itself.
    pub(super) fn quit(&self, path: &Path, force: bool) -> Result<String, String> {
        let name = self.name_of(path);
        let me = i32::try_from(std::process::id()).unwrap_or(-1);
        let pids = workspace::running_for_bundle(path);
        if self.is_own(path) || pids.contains(&me) {
            return Err(format!("{name} is Flick; quit it from the Flick menu"));
        }
        if pids.is_empty() {
            return Err(format!("{name} is not running"));
        }
        let send = if force { workspace::force_terminate } else { workspace::terminate };
        let asked = pids.into_iter().map(send).filter(|&ok| ok).count();
        match (asked, force) {
            (0, _) => Err(format!("Could not quit {name}")),
            (_, false) => Ok(format!("Asked {name} to quit")),
            (_, true) => Ok(format!("Force quit {name}")),
        }
    }

    /// The app at `path` runs, by the cached running apps (read now if there are none).
    fn is_running(&mut self, path: &Path) -> bool {
        let running =
            self.running.get_or_insert_with(|| workspace::running_bundles().into_iter().collect());
        running.contains(&workspace::canonical(path))
    }

    fn is_own(&self, path: &Path) -> bool {
        self.own.as_deref() == Some(path)
    }

    /// The indexed name of the app at `path`, else its file stem.
    pub(super) fn name_of(&self, path: &Path) -> String {
        let indexed = self.apps.iter().find(|a| a.path == path).map(|a| a.name.clone());
        indexed.unwrap_or_else(|| {
            path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use super::*;
    use crate::core::{ItemId, Module, test_cx};
    use crate::modules::apps::App;

    const FAKE: &str = "/nonexistent/flick/Fake.app";
    const OWN: &str = "/nonexistent/flick/Flick.app";

    /// Fake and Flick, both running by the cache (neither really runs: `quit` finds no pids).
    fn apps() -> Apps {
        let apps = vec![App { name: "Fake".into(), path: FAKE.into() }];
        let running = Some([FAKE, OWN].into_iter().map(PathBuf::from).collect());
        Apps { own: Some(OWN.into()), running, ..Apps::new(apps) }
    }

    fn keys(actions: &[Action]) -> Vec<&'static str> {
        actions.iter().map(|a| a.key).collect()
    }

    #[test]
    fn menu_offers_quit_on_apps_but_not_on_flick_or_non_paths() {
        let mut a = apps();
        test_cx("", |cx| {
            let menu = a.actions(&ItemId::new("app", FAKE), cx);
            assert_eq!(keys(&menu), [OPEN, REVEAL, QUIT, FORCE_QUIT]);
            assert_eq!(menu[1].title, "Show in Finder");
            assert_eq!(keys(&a.actions(&ItemId::new("app", OWN), cx)), [OPEN, REVEAL]);
            assert!(a.actions(&ItemId::new("app", "quit"), cx).is_empty());
        });
    }

    #[test]
    fn menu_hides_quit_for_apps_that_do_not_run() {
        let mut a = apps();
        a.running = Some(HashSet::new());
        test_cx("", |cx| {
            assert_eq!(keys(&a.actions(&ItemId::new("app", FAKE), cx)), [OPEN, REVEAL]);
        });
        // With no cache the menu reads the running apps: Finder always runs, Fake never.
        a.running = None;
        let finder = Path::new("/System/Library/CoreServices/Finder.app");
        assert!(keys(&a.menu(finder)).contains(&QUIT));
        assert!(a.running.is_some());
        assert_eq!(keys(&a.menu(Path::new(FAKE))), [OPEN, REVEAL]);
    }

    #[test]
    fn quit_refuses_apps_that_do_not_run_and_flick_itself() {
        // No real app is ever asked to quit: none of these paths can be running.
        let a = apps();
        for force in [false, true] {
            assert_eq!(a.quit(Path::new(FAKE), force).unwrap_err(), "Fake is not running");
            assert_eq!(
                a.quit(Path::new(OWN), force).unwrap_err(),
                "Flick is Flick; quit it from the Flick menu"
            );
        }
        let unindexed = Path::new("/nonexistent/flick/Other.app");
        assert_eq!(a.quit(unindexed, true).unwrap_err(), "Other is not running");
    }

    #[test]
    fn act_reports_quit_status_and_ignores_unknown_keys() {
        let mut a = apps();
        let id = ItemId::new("app", FAKE);
        test_cx("", |cx| {
            for key in [QUIT, FORCE_QUIT] {
                let status = a.act(&id, key, cx);
                assert!(matches!(status, Outcome::Stay(Some(s)) if s == "Fake is not running"));
            }
            assert!(matches!(a.act(&id, "nope", cx), Outcome::Stay(None)));
            let refusal = a.act(&id, UNINSTALL, cx);
            let want = "Cannot uninstall Fake: it has no bundle identifier";
            assert!(matches!(refusal, Outcome::Stay(Some(s)) if s == want));
        });
    }

    #[test]
    fn quit_verbs_find_the_app_by_name() {
        let mut a = apps();
        let args = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        test_cx("", |cx| {
            for verb in ["quit", "force-quit"] {
                let err = a.command(&args(&[verb, "fake"]), cx).unwrap_err();
                assert_eq!(err, "app: Fake is not running");
                let err = a.command(&args(&[verb, "No", "Such"]), cx).unwrap_err();
                assert_eq!(err, "app: no app named \"No Such\"");
            }
            for verb in ["quit", "force-quit", "reveal"] {
                let err = a.command(&args(&[verb]), cx).unwrap_err();
                assert_eq!(err, format!("app: unknown command \"{verb}\""));
            }
        });
    }
}
