//! Which config file to edit or open, and whether Flick may write it. With a per-host
//! overlay, a list such as `[[quicklink.links]]` lives in the overlay when the overlay sets
//! it (its list replaces the shared one), else in config.toml. A file that resolves into the
//! read-only Nix store (home-manager `source = ./file`) cannot be written: the error says so
//! and names the file, instead of a bare "Read-only file system".

use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

use super::overlay::overlay_path;
use super::parse;

/// The Nix store; files under it are read-only.
pub const NIX_STORE: &str = "/nix/store";

/// The file that holds the effective `[<module>] <key>`: the overlay for `host` when it
/// exists and sets that key (also through a legacy key), else `base`. An overlay that does
/// not load is chosen too, so the edit's error names it.
pub fn entries_file(base: &Path, host: Option<&str>, module: &str, key: &str) -> PathBuf {
    let Some(overlay) = host.map(|host| overlay_path(base, host)).filter(|p| p.exists()) else {
        return base.to_path_buf();
    };
    let sets = |text: String| {
        parse(&text).map_or(true, |c| {
            c.tables.get(module).and_then(Value::as_table).is_some_and(|t| t.contains_key(key))
        })
    };
    if fs::read_to_string(&overlay).map_or(true, sets) { overlay } else { base.to_path_buf() }
}

/// The files "Open Flick Config" opens: config.toml, then the overlay for `host` when it
/// exists. Each is resolved through symlinks, so an editor saves the real file (e.g. the
/// dotfiles copy a home-manager out-of-store link points at), not a link it would replace.
pub fn files_to_open(base: &Path, host: Option<&str>) -> Vec<PathBuf> {
    let overlay = host.map(|host| overlay_path(base, host)).filter(|p| p.exists());
    std::iter::once(base.to_path_buf())
        .chain(overlay)
        .map(|p| fs::canonicalize(&p).unwrap_or(p))
        .collect()
}

/// Whether `path` (already resolved) lies in the store at `store`.
pub fn in_store(path: &Path, store: &Path) -> bool {
    path.starts_with(store) || fs::canonicalize(store).is_ok_and(|s| path.starts_with(s))
}

/// `path` resolved through symlinks, ready to be replaced. Errors name `path` and say why it
/// cannot be written: it resolves into the store at `store`, or it is read-only.
pub fn writable_in(path: &Path, store: &Path) -> Result<PathBuf, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let target = fs::canonicalize(path).map_err(|e| at(&e))?;
    if in_store(&target, store) {
        return Err(at(&format!(
            "not edited, it links to {} in the read-only Nix store; edit the file your Nix \
             (home-manager) config builds it from, rebuild, then run Reload Flick Config",
            target.display()
        )));
    }
    let meta = fs::metadata(&target).map_err(|e| at(&e))?;
    if meta.permissions().readonly() {
        return Err(at(&format!("not edited, {} is read-only", target.display())));
    }
    Ok(target)
}

/// `writable_in` for the real Nix store.
pub fn writable(path: &Path) -> Result<PathBuf, String> {
    writable_in(path, Path::new(NIX_STORE))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    /// A fresh directory under the system temp dir, resolved (macOS /var is /private/var).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flick-target-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::canonicalize(&dir).unwrap()
    }

    const KOTA: &str = "[kota]\nhost = \"x\"\n";
    const LINKS: &str = "[[quicklink.links]]\nname = \"A\"\nurl = \"/\"\n";

    #[test]
    fn entries_live_where_the_effective_list_is_set() {
        let dir = scratch("entries");
        let base = dir.join("config.toml");
        let over = dir.join("config.mbp.toml");
        fs::write(&base, LINKS).unwrap();
        let file = |host| entries_file(&base, host, "quicklink", "links");
        // No overlay file, or no host: config.toml.
        assert_eq!(file(Some("mbp")), base);
        fs::write(&over, LINKS).unwrap();
        assert_eq!(file(None), base);
        // The overlay sets the list (also in legacy form): it holds the links.
        assert_eq!(file(Some("mbp")), over);
        fs::write(&over, "[[quicklinks]]\nname = \"A\"\nurl = \"/\"\n").unwrap();
        assert_eq!(file(Some("mbp")), over);
        // The overlay sets other keys of the table, or other tables: config.toml.
        fs::write(&over, "[quicklink]\nenabled = true\n[kota]\nhost = \"x\"\n").unwrap();
        assert_eq!(file(Some("mbp")), base);
        // A broken overlay is chosen, so the edit's error names it.
        fs::write(&over, "[quicklink").unwrap();
        assert_eq!(file(Some("mbp")), over);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_lists_the_config_then_the_overlay_resolved() {
        let dir = scratch("open");
        let real = dir.join("dotfiles.toml");
        let base = dir.join("config.toml");
        fs::write(&real, "").unwrap();
        symlink(&real, &base).unwrap();
        assert_eq!(files_to_open(&base, Some("mbp")), std::slice::from_ref(&real));
        let over = dir.join("config.mbp.toml");
        fs::write(&over, "").unwrap();
        assert_eq!(files_to_open(&base, Some("mbp")), [real.clone(), over]);
        assert_eq!(files_to_open(&base, None), [real]);
        // A missing config.toml is still listed, as is.
        let gone = dir.join("gone/config.toml");
        assert_eq!(files_to_open(&gone, Some("mbp")), [gone]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_link_into_the_store_is_refused_by_name() {
        let dir = scratch("store");
        let store = dir.join("nix/store");
        fs::create_dir_all(store.join("abc-home-manager-files")).unwrap();
        let built = store.join("abc-home-manager-files/config.toml");
        fs::write(&built, LINKS).unwrap();
        let link = dir.join("config.toml");
        symlink(&built, &link).unwrap();
        let err = writable_in(&link, &store).unwrap_err();
        assert!(err.starts_with(&format!("{}: not edited, it links to ", link.display())), "{err}");
        assert!(err.contains(&built.display().to_string()), "{err}");
        assert!(err.contains("Nix store"), "{err}");
        assert!(in_store(&built, &store));

        // A store link that points back out (an out-of-store link) writes its real file.
        let real = dir.join("dotfiles.toml");
        fs::write(&real, LINKS).unwrap();
        let out = store.join("abc-home-manager-files/config.mbp.toml");
        symlink(&real, &out).unwrap();
        let overlay = dir.join("config.mbp.toml");
        symlink(&out, &overlay).unwrap();
        assert_eq!(writable_in(&overlay, &store).unwrap(), real);
        assert!(!in_store(&real, &store));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Link names that a load of `base` plus `host`'s overlay sees.
    fn effective(base: &Path, host: &str) -> Vec<String> {
        let c = super::super::load_from(base, Some(host)).unwrap();
        let t = c.section("quicklink").unwrap().unwrap().get::<toml::Table>().unwrap();
        let links = t["links"].as_array().unwrap();
        links.iter().map(|l| l["name"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn edits_go_to_the_file_that_sets_the_links() {
        use super::super::edit::{Edit, edit_in};
        let dir = scratch("edit");
        let base = dir.join("config.toml");
        let over = dir.join("config.mbp.toml");
        let add = |name: &str| Edit::Append(vec![("name", name.into()), ("url", "/".into())]);
        let edit = |e: &Edit| edit_in(&base, Some("mbp"), "quicklink", "links", e);
        fs::write(&base, LINKS).unwrap();
        fs::write(&over, KOTA).unwrap();
        edit(&add("B")).unwrap();
        assert_eq!(effective(&base, "mbp"), ["A", "B"]);
        assert_eq!(fs::read_to_string(&over).unwrap(), KOTA);

        // The overlay's list replaces the shared one, so edits go there.
        let mine = "[[quicklink.links]]\nname = \"Mine\"\nurl = \"~/\"\n";
        fs::write(&over, mine).unwrap();
        edit(&add("C")).unwrap();
        assert_eq!(effective(&base, "mbp"), ["Mine", "C"]);
        edit(&Edit::Remove { name: "Mine".into() }).unwrap();
        assert_eq!(effective(&base, "mbp"), ["C"]);
        let err = edit(&Edit::Remove { name: "A".into() }).unwrap_err();
        assert!(err.starts_with(&over.display().to_string()), "{err}");
        assert_eq!(
            fs::read_to_string(&base).unwrap(),
            format!("{LINKS}\n[[quicklink.links]]\nname = \"B\"\nurl = \"/\"\n")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_only_and_missing_files_are_refused() {
        let dir = scratch("readonly");
        let path = dir.join("config.toml");
        assert!(writable(&path).unwrap_err().starts_with(&path.display().to_string()));
        fs::write(&path, "").unwrap();
        assert_eq!(writable(&path).unwrap(), path);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let err = writable(&path).unwrap_err();
        assert!(err.ends_with("is read-only"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }
}
