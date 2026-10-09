//! Leftover finder and system-app protection for uninstall. Pure: the home folder, bundle id
//! and Flick's own bundle are parameters, and nothing here deletes anything.
//!
//! Matching is conservative: exact, case-sensitive bundle-id names only, direct children of a
//! fixed set of `~/Library` folders, and never through a symlink. A missed leftover is harmless;
//! a wrong one would trash another app's data.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// The `~/Library` folders searched for leftovers. Nothing outside them is ever matched.
pub const ROOTS: [&str; 10] = [
    "Application Support",
    "Caches",
    "Preferences",
    "Containers",
    "Group Containers",
    "Saved Application State",
    "LaunchAgents",
    "Logs",
    "HTTPStorages",
    "WebKit",
];

/// Entries a size walk visits before it stops and reports a lower bound.
const WALK_LIMIT: usize = 200_000;

/// A leftover path and its size.
#[derive(Debug, PartialEq)]
pub struct Leftover {
    pub path: PathBuf,
    pub size: Size,
}

/// Bytes of the regular files under a path. `capped`: the walk stopped early, so `bytes` is a
/// lower bound (show it as ">= X").
#[derive(Debug, PartialEq, Clone, Copy, Default)]
pub struct Size {
    pub bytes: u64,
    pub capped: bool,
}

/// Leftovers of the app with bundle id `bid` under `home`, sorted by path, with sizes.
pub fn find(home: &Path, bid: &str) -> Vec<Leftover> {
    matches(home, bid).into_iter().map(|path| Leftover { size: size(&path), path }).collect()
}

/// The leftover paths of `bid` under `home`, sorted. Each is a direct child of a canonical
/// `~/Library/<root>` (or `Preferences/ByHost`) that is a real folder, and is not a symlink.
pub fn matches(home: &Path, bid: &str) -> Vec<PathBuf> {
    if !valid_bid(bid) {
        return vec![];
    }
    let Ok(library) = home.join("Library").canonicalize() else { return vec![] };
    let mut out = vec![];
    for root in ROOTS {
        let dir = library.join(root);
        scan(&dir, |name| name_matches(root, name, bid), &mut out);
        if root == "Preferences" {
            scan(&dir.join("ByHost"), |name| by_host_matches(name, bid), &mut out);
        }
    }
    out.sort();
    out
}

/// Re-check one path just before it is trashed: still a current match for `bid`.
pub fn still_matches(home: &Path, bid: &str, path: &Path) -> bool {
    matches(home, bid).iter().any(|p| p == path)
}

/// Push the entries of `dir` whose names pass `keep` and that are not symlinks. `dir` itself
/// must be a real folder, not a symlink to one.
fn scan(dir: &Path, keep: impl Fn(&str) -> bool, out: &mut Vec<PathBuf>) {
    if !fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let path = dir.join(name);
        let real = fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_symlink());
        if real && keep(name) {
            out.push(path);
        }
    }
}

/// A bundle id that can only ever name one directory entry: no separators, no `.`/`..`.
fn valid_bid(bid: &str) -> bool {
    let chars = bid.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    chars && bid.contains(|c: char| c.is_ascii_alphanumeric()) && !bid.starts_with('.')
}

/// Whether `name` in `~/Library/<root>` belongs to `bid`.
fn name_matches(root: &str, name: &str, bid: &str) -> bool {
    if name == bid {
        return true;
    }
    match root {
        "Preferences" | "LaunchAgents" => name.strip_suffix(".plist") == Some(bid),
        "Saved Application State" => name.strip_suffix(".savedState") == Some(bid),
        "HTTPStorages" => name.strip_suffix(".binarycookies") == Some(bid),
        "Group Containers" => name
            .strip_suffix(bid)
            .and_then(|p| p.strip_suffix('.'))
            .is_some_and(|p| p == "group" || is_team_id(p)),
        _ => false,
    }
}

/// `<bid>.<hardware UUID>.plist` in `Preferences/ByHost`.
fn by_host_matches(name: &str, bid: &str) -> bool {
    let uuid = name.strip_prefix(bid).and_then(|r| r.strip_prefix('.'));
    uuid.and_then(|r| r.strip_suffix(".plist")).is_some_and(is_uuid)
}

/// An Apple developer team id: ten uppercase letters or digits.
fn is_team_id(s: &str) -> bool {
    s.len() == 10 && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// `8-4-4-4-12` hex digits.
fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    let lens = groups.iter().map(|g| g.len()).collect::<Vec<_>>();
    lens == [8, 4, 4, 4, 12] && groups.iter().all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Size of `path` without following symlinks (a symlink counts as 0 bytes).
pub fn size(path: &Path) -> Size {
    walk(path, WALK_LIMIT)
}

fn walk(path: &Path, limit: usize) -> Size {
    let mut size = Size::default();
    let mut stack = vec![path.to_path_buf()];
    let mut seen = 0;
    while let Some(path) = stack.pop() {
        if seen == limit {
            size.capped = true;
            break;
        }
        seen += 1;
        let Ok(meta) = fs::symlink_metadata(&path) else { continue };
        if meta.is_file() {
            size.bytes += meta.len();
        } else if meta.is_dir() {
            let Ok(entries) = fs::read_dir(&path) else { continue };
            stack.extend(entries.flatten().map(|e| e.path()));
        }
    }
    size
}

/// Why the app at `app_path` must not be uninstalled, or `None` when it may be. `bid` is its
/// bundle id, `own` Flick's own bundle (if Flick runs from one), `home` the user's home.
pub fn protected(
    app_path: &Path,
    bid: Option<&str>,
    own: Option<&Path>,
    home: &Path,
) -> Option<&'static str> {
    let bid = bid.unwrap_or("");
    if app_path.starts_with("/System") || app_path.starts_with("/Library/Apple") {
        Some("it is part of macOS")
    } else if bid.is_empty() {
        Some("it has no bundle identifier")
    } else if bid.starts_with("com.apple.") {
        Some("it is an Apple app")
    } else if own.is_some_and(|own| own == app_path) {
        Some("it is Flick")
    } else if !in_app_folder(app_path, home) {
        Some("it is not in /Applications or ~/Applications")
    } else {
        None
    }
}

/// `path` is a plain absolute path directly in, or one folder below, `/Applications` or
/// `~/Applications` (the folders the app scan reads).
fn in_app_folder(path: &Path, home: &Path) -> bool {
    let plain = path.is_absolute()
        && path.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_)));
    let home_apps = home.join("Applications");
    let roots = [Path::new("/Applications"), home_apps.as_path()];
    let parent = path.parent();
    let grandparent = parent.and_then(Path::parent);
    plain && roots.iter().any(|r| parent == Some(r) || grandparent == Some(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BID: &str = "dev.flick.test";
    const UUID: &str = "0A1B2C3D-4E5F-6789-ABCD-EF0123456789";

    /// A temp home folder, removed on drop. Test-only cleanup; the finder itself never deletes.
    struct Home(PathBuf);

    impl Home {
        fn new(name: &str) -> Home {
            let dir = std::env::temp_dir().join(format!("flk-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("Library")).unwrap();
            Home(dir.canonicalize().unwrap())
        }

        /// Write `bytes` bytes to `Library/<rel>`, creating parents.
        fn file(&self, rel: &str, bytes: usize) -> PathBuf {
            let path = self.0.join("Library").join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![b'x'; bytes]).unwrap();
            path
        }

        fn lib(&self, rel: &str) -> PathBuf {
            self.0.join("Library").join(rel)
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_exactly_the_bundle_id_matches_with_sizes() {
        let home = Home::new("find");
        let h = &home;
        h.file("Caches/dev.flick.test/a", 10);
        h.file("Caches/dev.flick.test/sub/b", 5);
        h.file("Preferences/dev.flick.test.plist", 7);
        h.file(&format!("Preferences/ByHost/dev.flick.test.{UUID}.plist"), 3);
        h.file("Saved Application State/dev.flick.test.savedState/window_1.data", 4);
        h.file("HTTPStorages/dev.flick.test.binarycookies", 2);
        h.file("HTTPStorages/dev.flick.test/c", 1);
        h.file("Group Containers/ABCDE12345.dev.flick.test/d", 6);
        h.file("Group Containers/group.dev.flick.test/e", 8);
        h.file("LaunchAgents/dev.flick.test.plist", 9);
        h.file("Containers/dev.flick.test/f", 11);
        h.file("Application Support/dev.flick.test/g", 12);
        h.file("Logs/dev.flick.test/h", 13);
        h.file("WebKit/dev.flick.test/i", 14);
        // Not matches: near misses, other case, app name, other suffix placement, other roots.
        for decoy in [
            "Caches/dev.flick.test2/x",
            "Caches/dev.flick.test.helper/x",
            "Caches/Flick Test/x",
            "Caches/dev.flick.test.plist",
            "Application Support/dev.flick.test.savedState/x",
            "Logs/dev.flick.test.binarycookies",
            "Preferences/dev.flick.test.helper.plist",
            "Preferences/dev.flick.test.savedState",
            "Preferences/ByHost/dev.flick.test.plist",
            "Preferences/ByHost/dev.flick.test.helper.0A1B2C3D-4E5F-6789-ABCD-EF0123456789.plist",
            "Preferences/ByHost/dev.flick.test.0A1B2C3D-4E5F-6789-ABCD-EF012345678Z.plist",
            "Preferences/ByHost/dev.flick.test.ABCDEF.plist",
            "Group Containers/ABCDE1234.dev.flick.test/x",
            "Group Containers/ABCDE12345dev.flick.test/x",
            "Group Containers/ABCDE12345.dev.flick.test2/x",
            "Mail/dev.flick.test/x",
            "Cookies/dev.flick.test.binarycookies",
        ] {
            h.file(decoy, 1);
        }
        // A symlink named exactly like the bundle id, pointing outside ~/Library.
        let outside = h.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, h.lib("Logs/dev.flick.test2")).unwrap();
        std::os::unix::fs::symlink(&outside, h.lib("Containers/link")).unwrap();
        fs::create_dir_all(h.lib("WebKit/real")).unwrap();
        std::os::unix::fs::symlink(&outside, h.lib("WebKit/real/dev.flick.test")).unwrap();

        let found = find(&h.0, BID);
        let rel = |l: &Leftover| l.path.strip_prefix(h.lib("")).unwrap().display().to_string();
        let got: Vec<(String, u64)> = found.iter().map(|l| (rel(l), l.size.bytes)).collect();
        let want = [
            ("Application Support/dev.flick.test", 12),
            ("Caches/dev.flick.test", 15),
            ("Containers/dev.flick.test", 11),
            ("Group Containers/ABCDE12345.dev.flick.test", 6),
            ("Group Containers/group.dev.flick.test", 8),
            ("HTTPStorages/dev.flick.test", 1),
            ("HTTPStorages/dev.flick.test.binarycookies", 2),
            ("LaunchAgents/dev.flick.test.plist", 9),
            ("Logs/dev.flick.test", 13),
            ("Preferences/ByHost/dev.flick.test.0A1B2C3D-4E5F-6789-ABCD-EF0123456789.plist", 3),
            ("Preferences/dev.flick.test.plist", 7),
            ("Saved Application State/dev.flick.test.savedState", 4),
            ("WebKit/dev.flick.test", 14),
        ];
        let want: Vec<(String, u64)> = want.iter().map(|(p, n)| ((*p).to_string(), *n)).collect();
        assert_eq!(got, want);
        assert!(found.iter().all(|l| !l.size.capped));
        assert!(still_matches(&h.0, BID, &h.lib("Caches/dev.flick.test")));
        assert!(!still_matches(&h.0, BID, &h.lib("Caches/dev.flick.test2")));
        assert!(!still_matches(&h.0, "dev.flick.other", &h.lib("Caches/dev.flick.test")));
    }

    #[test]
    fn symlinks_are_never_matched_or_followed() {
        let home = Home::new("links");
        let h = &home;
        let outside = h.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("big"), vec![0; 100]).unwrap();
        // A symlink entry named like the bundle id is skipped.
        fs::create_dir_all(h.lib("Caches")).unwrap();
        std::os::unix::fs::symlink(&outside, h.lib("Caches/dev.flick.test")).unwrap();
        // A symlink inside a real match is not followed when sizing.
        h.file("Logs/dev.flick.test/own", 4);
        std::os::unix::fs::symlink(&outside, h.lib("Logs/dev.flick.test/link")).unwrap();
        // A root that is itself a symlink is not searched.
        fs::create_dir_all(outside.join("dev.flick.test")).unwrap();
        std::os::unix::fs::symlink(&outside, h.lib("Containers")).unwrap();
        // Neither is a symlinked ByHost folder.
        fs::write(outside.join(format!("dev.flick.test.{UUID}.plist")), b"x").unwrap();
        fs::create_dir_all(h.lib("Preferences")).unwrap();
        std::os::unix::fs::symlink(&outside, h.lib("Preferences/ByHost")).unwrap();

        let found = find(&h.0, BID);
        assert_eq!(found, [Leftover { path: h.lib("Logs/dev.flick.test"), size: size_of(4) }]);
        assert!(!still_matches(&h.0, BID, &h.lib("Caches/dev.flick.test")));
    }

    /// Case variants cannot sit next to the real names on case-insensitive APFS, so the
    /// case rule is tested on names.
    #[test]
    fn names_match_case_sensitively() {
        assert!(name_matches("Caches", BID, BID));
        assert!(!name_matches("Caches", "DEV.FLICK.TEST", BID));
        assert!(!name_matches("Preferences", "dev.flick.test.PLIST", BID));
        assert!(!name_matches("Group Containers", "abcde12345.dev.flick.test", BID));
        assert!(!name_matches("Group Containers", "GROUP.dev.flick.test", BID));
        assert!(!by_host_matches(&format!("DEV.FLICK.TEST.{UUID}.plist"), BID));
    }

    fn size_of(bytes: u64) -> Size {
        Size { bytes, capped: false }
    }

    #[test]
    fn unusable_bundle_ids_and_homes_match_nothing() {
        let home = Home::new("bids");
        let h = &home;
        h.file("Caches/x", 1);
        h.file("Caches/dev.flick.test/a", 1);
        for bid in ["", ".", "..", "-", "a/b", "../Caches", ".hidden", "dev flick"] {
            assert_eq!(matches(&h.0, bid), Vec::<PathBuf>::new(), "{bid:?}");
        }
        assert_eq!(matches(&h.0.join("nope"), BID), Vec::<PathBuf>::new());
        assert_eq!(matches(&h.0, "dev.flick.test").len(), 1);
    }

    #[test]
    fn size_walk_counts_files_and_stops_at_its_limit() {
        let home = Home::new("walk");
        let h = &home;
        let dir = h.lib("Caches/w");
        for i in 0..5 {
            h.file(&format!("Caches/w/{i}"), 10);
        }
        assert_eq!(size(&dir), size_of(50));
        assert_eq!(size(&h.lib("Caches/w/0")), size_of(10));
        assert_eq!(size(&h.lib("missing")), size_of(0));
        let capped = walk(&dir, 3);
        assert!(capped.capped && capped.bytes == 20, "{capped:?}");
        assert_eq!(walk(&dir, 6), size_of(50));
    }

    #[test]
    fn system_apple_own_unidentified_and_stray_apps_are_protected() {
        let home = Path::new("/Users/me");
        let own = Path::new("/Applications/Flick.app");
        let p = |path: &str, bid: Option<&str>| protected(Path::new(path), bid, Some(own), home);
        assert_eq!(p("/System/Applications/Safari.app", Some("com.x")), Some("it is part of macOS"));
        assert_eq!(p("/Library/Apple/System/X.app", Some("dev.x")), Some("it is part of macOS"));
        assert_eq!(p("/Applications/Safari.app", Some("com.apple.Safari")), Some("it is an Apple app"));
        assert_eq!(p("/Applications/Foo.app", None), Some("it has no bundle identifier"));
        assert_eq!(p("/Applications/Foo.app", Some("")), Some("it has no bundle identifier"));
        assert_eq!(p("/Applications/Flick.app", Some("dev.flick")), Some("it is Flick"));
        let stray = Some("it is not in /Applications or ~/Applications");
        for path in [
            "/usr/local/foo.app",
            "/Applications/a/b/Foo.app",
            "/Applications/../usr/Foo.app",
            "Applications/Foo.app",
            "/Applications",
            "/",
        ] {
            assert_eq!(p(path, Some("dev.foo.bar")), stray, "{path}");
        }
        for path in [
            "/Applications/Foo.app",
            "/Applications/Utilities/Foo.app",
            "/Users/me/Applications/Foo.app",
            "/Users/me/Applications/Tools/Foo.app",
        ] {
            assert_eq!(p(path, Some("dev.foo.bar")), None, "{path}");
        }
        assert_eq!(protected(own, Some("dev.flick"), None, home), None);
    }
}
