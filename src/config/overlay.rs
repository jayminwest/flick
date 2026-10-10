//! Per-host overlay: `config.<host>.toml` next to config.toml, where `<host>` is this Mac's
//! `LocalHostName` in lowercase. It is a plain deep merge over config.toml: tables merge key by
//! key, any other value (a string, a number, an array, an array of tables such as
//! `[[sys.machine]]`) replaces the base value. An overlay cannot delete a base key. Without
//! an overlay file, loading is exactly the single-file load.

use std::path::{Path, PathBuf};

use toml::{Table, Value};

use super::{Config, from_table, parse};

/// The overlay for `host` next to `base`: `<base dir>/config.<host>.toml`.
pub fn overlay_path(base: &Path, host: &str) -> PathBuf {
    base.with_file_name(format!("config.{host}.toml"))
}

/// A host name as overlay files use it: trimmed, a trailing `.local` dropped, lowercase.
/// `None` when empty or holding anything but letters, digits, `-`, `_` and `.` (so it can
/// never name a path outside the config directory).
pub fn normalize(raw: &str) -> Option<String> {
    let name = raw.trim().to_ascii_lowercase();
    let name = name.strip_suffix(".local").unwrap_or(&name);
    let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    (!name.is_empty() && !name.starts_with('.') && name.chars().all(ok)).then(|| name.into())
}

/// This Mac's overlay host: `$FLICK_HOST_NAME` when set (empty means no overlay), else what
/// `local` reports (`LocalHostName`). Both pass through `normalize`.
pub fn host_from(env: Option<&str>, local: &dyn Fn() -> Option<String>) -> Option<String> {
    match env {
        Some(name) => normalize(name),
        None => local().as_deref().and_then(normalize),
    }
}

/// Merge `overlay` into `base`: tables recursively, every other value replaced.
pub fn merge(base: &mut Table, overlay: Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(into)), Value::Table(from)) => merge(into, from),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Load `text` (read from `base`), then the overlay for `host` when that file exists.
/// `exists`/`read` stand in for the file system. Errors name the file they come from.
pub fn load(
    base: &Path,
    text: &str,
    host: Option<&str>,
    exists: &dyn Fn(&Path) -> bool,
    read: &dyn Fn(&Path) -> std::io::Result<String>,
) -> Result<Config, String> {
    let at = |path: &Path, e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut config = parse(text).map_err(|e| at(base, &e))?;
    let Some(path) = host.map(|host| overlay_path(base, host)).filter(|p| exists(p)) else {
        return Ok(config);
    };
    let over: Table = read(&path)
        .map_err(|e| at(&path, &e))
        .and_then(|text| toml::from_str(&text).map_err(|e| at(&path, &e)))?;
    let sets_hotkey = over.contains_key("hotkey");
    let over = from_table(over).map_err(|e| at(&path, &e))?;
    if sets_hotkey {
        config.hotkey = over.hotkey;
    }
    merge(&mut config.tables, over.tables);
    config.overlay = Some(path);
    Ok(config)
}

impl Config {
    /// The files this config came from, for error messages: config.toml, plus the overlay.
    pub fn files(&self, base: &Path) -> String {
        match &self.overlay {
            None => base.display().to_string(),
            Some(overlay) => format!("{} + {}", base.display(), overlay.display()),
        }
    }
}

/// `flick config path`: the config file and this Mac's overlay, each marked when missing.
pub fn report(base: &Path, host: Option<&str>, exists: &dyn Fn(&Path) -> bool) -> String {
    let overlay = match host {
        Some(host) => {
            let path = overlay_path(base, host);
            let missing = if exists(&path) { "" } else { ", missing" };
            format!("{} (host {host}{missing})", path.display())
        }
        None => "none (no host name)".into(),
    };
    let missing = if exists(base) { "" } else { " (missing)" };
    format!("config   {}{missing}\noverlay  {overlay}", base.display())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::io;

    use super::*;

    fn table(text: &str) -> Table {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn the_overlay_sits_next_to_the_config() {
        let base = Path::new("/c/flick/my.toml");
        assert_eq!(overlay_path(base, "mbp-server"), Path::new("/c/flick/config.mbp-server.toml"));
    }

    #[test]
    fn host_names_are_lowercase_without_local() {
        assert_eq!(normalize("Jaymins-MacBook-Pro").as_deref(), Some("jaymins-macbook-pro"));
        assert_eq!(normalize(" mbp-server.local\n").as_deref(), Some("mbp-server"));
        assert_eq!(normalize("MBP-Server.LOCAL").as_deref(), Some("mbp-server"));
        assert_eq!(normalize("a_b.c").as_deref(), Some("a_b.c"));
        for bad in ["", "  ", ".local", "../x", "a/b", ".hidden", "a b", "é"] {
            assert_eq!(normalize(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_env_name_wins_and_empty_turns_overlays_off() {
        let asked = Cell::new(false);
        let local = || {
            asked.set(true);
            Some("Mac-Studio".to_string())
        };
        assert_eq!(host_from(Some("Laptop"), &local).as_deref(), Some("laptop"));
        assert_eq!(host_from(Some(""), &local), None);
        assert!(!asked.get());
        assert_eq!(host_from(None, &local).as_deref(), Some("mac-studio"));
        assert_eq!(host_from(None, &|| None), None);
    }

    #[test]
    fn tables_merge_and_other_values_replace() {
        let mut base = table(
            r#"
            top = 1
            list = [1, 2]
            [kota]
            host = "a"
            keep = true
            [kota.deep]
            x = 1
            y = 2
            [[sys.machine]]
            name = "laptop"
            [[sys.machine]]
            name = "server"
            "#,
        );
        let over = table(
            r#"
            top = "two"
            list = [3]
            new = 4
            [kota]
            host = "b"
            [kota.deep]
            y = 3
            [[sys.machine]]
            name = "only"
            "#,
        );
        merge(&mut base, over);
        let want = table(
            r#"
            top = "two"
            list = [3]
            new = 4
            [kota]
            host = "b"
            keep = true
            [kota.deep]
            x = 1
            y = 3
            [[sys.machine]]
            name = "only"
            "#,
        );
        assert_eq!(base, want);
    }

    #[test]
    fn a_table_and_a_value_replace_each_other() {
        let mut base = table("a = 1\n[b]\nx = 1");
        merge(&mut base, table("b = 2\n[a]\ny = 1"));
        assert_eq!(base, table("b = 2\n[a]\ny = 1"));
    }

    /// An in-memory file system.
    struct Files(BTreeMap<PathBuf, Result<String, ()>>);

    impl Files {
        fn exists(&self, p: &Path) -> bool {
            self.0.contains_key(p)
        }

        fn read(&self, p: &Path) -> io::Result<String> {
            match self.0.get(p) {
                Some(Ok(text)) => Ok(text.clone()),
                _ => Err(io::Error::other("unreadable")),
            }
        }

        fn load(&self, text: &str, host: Option<&str>) -> Result<Config, String> {
            load(BASE.as_ref(), text, host, &|p| self.exists(p), &|p| self.read(p))
        }
    }

    const BASE: &str = "/c/config.toml";
    const OVER: &str = "/c/config.mbp.toml";

    fn files(overlay: Result<&str, ()>) -> Files {
        Files(BTreeMap::from([(PathBuf::from(OVER), overlay.map(String::from))]))
    }

    fn kota(c: &Config) -> Table {
        c.section("kota").unwrap().unwrap().get().unwrap()
    }

    #[test]
    fn without_an_overlay_the_load_is_the_plain_parse() {
        let text = "hotkey = \"cmd+K\"\nwindows_hotkey = \"alt+Tab\"\n[kota]\nhost = \"a\"";
        let plain = parse(text).unwrap();
        for c in [Files(BTreeMap::new()).load(text, Some("mbp")), files(Ok("")).load(text, None)] {
            let c = c.unwrap();
            assert_eq!(format!("{c:?}"), format!("{plain:?}"));
            assert_eq!(c.files(BASE.as_ref()), BASE);
        }
        let err = files(Ok("")).load("hotkey = 1", Some("mbp")).unwrap_err();
        assert!(err.starts_with("/c/config.toml: "), "{err}");
    }

    #[test]
    fn the_overlay_merges_over_the_base() {
        let base = "hotkey = \"cmd+K\"\n[kota]\nhost = \"a\"\nkeep = 1\n[clip]\nenabled = false";
        let c = files(Ok("[kota]\nhost = \"b\"\n[remote]\npeers = [\"x\"]"))
            .load(base, Some("mbp"))
            .unwrap();
        assert_eq!(c.hotkey, "cmd+K");
        assert_eq!(kota(&c), table("host = \"b\"\nkeep = 1"));
        assert!(c.section("clip").unwrap().is_none());
        assert!(c.section("remote").unwrap().is_some());
        assert_eq!(c.files(BASE.as_ref()), "/c/config.toml + /c/config.mbp.toml");
    }

    #[test]
    fn the_overlay_sets_the_hotkey_only_when_it_has_one() {
        let c = files(Ok("hotkey = \"cmd+J\"")).load("hotkey = \"cmd+K\"", Some("mbp")).unwrap();
        assert_eq!(c.hotkey, "cmd+J");
        let c = files(Ok("hotkey = \"cmd+J\"")).load("", Some("mbp")).unwrap();
        assert_eq!(c.hotkey, "cmd+J");
    }

    #[test]
    fn legacy_keys_map_in_each_file_before_the_merge() {
        // The overlay's links replace the base's legacy links instead of joining them.
        let base = "[[quicklinks]]\nname = \"Old\"\nurl = \"o\"";
        let over = "[[quicklink.links]]\nname = \"New\"\nurl = \"n\"";
        let c = files(Ok(over)).load(base, Some("mbp")).unwrap();
        let q: Table = c.section("quicklink").unwrap().unwrap().get().unwrap();
        assert_eq!(q, table(over)["quicklink"].as_table().unwrap().clone());
    }

    #[test]
    fn overlay_errors_name_the_overlay() {
        for (overlay, want) in [
            (Err(()), "unreadable"),
            (Ok("[kota"), "unclosed table"),
            (Ok("hotkey = 1"), "invalid type"),
            (Ok("quicklinks = []\n[quicklink]\nlinks = 1"), "both set"),
        ] {
            let err = files(overlay).load("", Some("mbp")).unwrap_err();
            assert!(err.starts_with("/c/config.mbp.toml: "), "{err}");
            assert!(err.contains(want), "{err}");
        }
    }

    #[test]
    fn the_report_shows_both_paths() {
        let base = Path::new(BASE);
        assert_eq!(
            report(base, Some("mbp"), &|_| true),
            "config   /c/config.toml\noverlay  /c/config.mbp.toml (host mbp)"
        );
        assert_eq!(
            report(base, Some("mbp"), &|p| p == base),
            "config   /c/config.toml\noverlay  /c/config.mbp.toml (host mbp, missing)"
        );
        assert_eq!(
            report(base, None, &|_| false),
            "config   /c/config.toml (missing)\noverlay  none (no host name)"
        );
    }
}
