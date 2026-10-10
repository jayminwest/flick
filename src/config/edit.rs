//! Edit the `[[<module>.<key>]]` entries of config.toml, or of the per-host overlay when that
//! is where they live (`target::entries_file`), in place. Everything outside the
//! edited entry (comments, blank lines, key order) stays byte for byte. Writes are atomic and
//! never leave a file that `config::parse` rejects.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use super::{LEGACY, config_path, host_name, parse, target, write_default};

/// One entry: `(field, value)` pairs in file order. Leave a field out to drop it.
pub type Entry = Vec<(&'static str, String)>;

/// A change to the `[[<module>.<key>]]` array. Entries are found by their `name` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Add an entry after the last `[[<module>.<key>]]` one (never in a legacy table).
    Append(Entry),
    /// Swap the fields of entry `name`, here or in its legacy table. Its comments stay.
    Replace { name: String, entry: Entry },
    /// Delete entry `name`, here or in its legacy table.
    Remove { name: String },
}

/// Apply `edit` to the file that holds the entries: this Mac's overlay when it sets them,
/// else config.toml (written with the default first when there is none).
pub fn edit_entries(module: &str, key: &str, edit: &Edit) -> Result<(), String> {
    edit_in(&config_path(), host_name().as_deref(), module, key, edit)
}

/// `edit_entries` for config file `base` and the overlay of `host` next to it.
pub fn edit_in(
    base: &Path,
    host: Option<&str>,
    module: &str,
    key: &str,
    edit: &Edit,
) -> Result<(), String> {
    edit_file(&target::entries_file(base, host, module, key), module, key, edit)
}

/// Re-read `path` (never a cached copy, so a hand edit is not lost), edit it and replace it
/// atomically. A symlink is written through to its target, unless that is in the Nix store
/// or read-only (`target::writable`).
pub fn edit_file(path: &Path, module: &str, key: &str, edit: &Edit) -> Result<(), String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    if !path.exists() && fs::symlink_metadata(path).is_err() {
        write_default(path);
    }
    let target = target::writable(path)?;
    let text = fs::read_to_string(&target).map_err(|e| at(&e))?;
    let text = edit_text(&text, module, key, edit).map_err(|e| at(&e))?;
    write_atomic(&target, &text).map_err(|e| at(&e))
}

/// `text` with `edit` applied. Refuses a file that does not load, and an edit that would
/// leave one.
fn edit_text(text: &str, module: &str, key: &str, edit: &Edit) -> Result<String, String> {
    parse(text).map_err(|e| format!("not edited, the file does not load: {e}"))?;
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let what = format!("{module}.{key}");
    match edit {
        Edit::Append(entry) => append(&mut doc, module, key, entry)
            .ok_or_else(|| format!("{module}: expected a [{module}] table of [[{what}]]"))?,
        Edit::Replace { name, entry } => {
            let (mut entries, i) = find(&mut doc, module, key, name)
                .ok_or_else(|| format!("no [[{what}]] entry named \"{name}\""))?;
            entries.replace(i, entry);
        }
        Edit::Remove { name } => {
            let (mut entries, i) = find(&mut doc, module, key, name)
                .ok_or_else(|| format!("no [[{what}]] entry named \"{name}\""))?;
            entries.remove(i);
        }
    }
    let text = doc.to_string();
    parse(&text).map_err(|e| format!("not edited, the result would not load: {e}"))?;
    Ok(text)
}

/// An array of entries: `[[a.b]]` tables or an inline `b = [{ ... }]`.
enum Entries<'a> {
    Tables(&'a mut ArrayOfTables),
    Inline(&'a mut Array),
}

impl<'a> Entries<'a> {
    fn of(item: &'a mut Item) -> Option<Self> {
        match item {
            Item::ArrayOfTables(tables) => Some(Self::Tables(tables)),
            Item::Value(Value::Array(array)) => Some(Self::Inline(array)),
            _ => None,
        }
    }

    fn push(&mut self, entry: &Entry) {
        match self {
            Self::Tables(tables) => {
                let mut table = Table::new();
                fill(&mut table, entry);
                tables.push(table);
            }
            Self::Inline(array) => {
                let mut table = InlineTable::new();
                fill(&mut table, entry);
                array.push(table);
            }
        }
    }

    fn replace(&mut self, i: usize, entry: &Entry) {
        match self {
            Self::Tables(tables) => tables.get_mut(i).map(|t| fill(t, entry)),
            Self::Inline(array) => {
                array.get_mut(i).and_then(Value::as_inline_table_mut).map(|t| fill(t, entry))
            }
        };
    }

    /// The comment above the first entry usually heads the whole array, so the first entry's
    /// decor (comment, spacing) moves to the next one.
    fn remove(&mut self, i: usize) {
        match self {
            Self::Tables(tables) => {
                let gone = tables.remove(i);
                if let (0, Some(next)) = (i, tables.get_mut(0)) {
                    *next.decor_mut() = gone.decor().clone();
                }
            }
            Self::Inline(array) => {
                let gone = array.remove(i);
                if let (0, Some(next)) = (i, array.get_mut(0)) {
                    *next.decor_mut() = gone.decor().clone();
                }
            }
        }
    }
}

/// Set `table` to exactly `entry`'s fields. Existing fields keep their place and comments.
fn fill(table: &mut dyn TableLike, entry: &Entry) {
    let dropped: Vec<String> = table
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !entry.iter().any(|(field, _)| field == k))
        .collect();
    for k in dropped {
        table.remove(&k);
    }
    for (field, text) in entry {
        let mut value = Value::from(text.as_str());
        match table.get_mut(field).and_then(Item::as_value_mut) {
            Some(old) => {
                *value.decor_mut() = old.decor().clone();
                *old = value;
            }
            None => {
                table.insert(field, Item::Value(value));
            }
        }
    }
}

/// Push `entry` onto `[[module.key]]`, creating the array (and an implicit `[module]`) when
/// missing. `None` when `module` or `module.key` is something else.
fn append(doc: &mut DocumentMut, module: &str, key: &str, entry: &Entry) -> Option<()> {
    let section = doc.entry(module).or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    let item = section
        .as_table_like_mut()?
        .entry(key)
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    Entries::of(item)?.push(entry);
    Some(())
}

/// Entry `name` in `[[module.key]]`, else in the legacy top-level array `LEGACY` maps there.
fn find<'a>(
    doc: &'a mut DocumentMut,
    module: &str,
    key: &str,
    name: &str,
) -> Option<(Entries<'a>, usize)> {
    let here = doc.get(module).and_then(Item::as_table_like).and_then(|t| t.get(key));
    if let Some(i) = here.and_then(|item| position(item, name)) {
        let item = doc.get_mut(module)?.as_table_like_mut()?.get_mut(key)?;
        return Some((Entries::of(item)?, i));
    }
    let (old, ..) = LEGACY.iter().find(|(_, m, k)| *m == module && *k == key)?;
    let i = position(doc.get(old)?, name)?;
    Some((Entries::of(doc.get_mut(old)?)?, i))
}

/// The index of the entry whose `name` is `name` in an array of tables or inline tables.
fn position(item: &Item, name: &str) -> Option<usize> {
    let named = |t: &dyn TableLike| t.get("name").and_then(Item::as_str) == Some(name);
    match item {
        Item::ArrayOfTables(tables) => tables.iter().position(|t| named(t)),
        Item::Value(Value::Array(array)) => {
            array.iter().position(|v| v.as_inline_table().is_some_and(|t| named(t)))
        }
        _ => None,
    }
}

/// Replace `target` with `text`: a temp file in the same directory, fsync, rename. The new
/// file keeps the old one's permissions.
fn write_atomic(target: &Path, text: &str) -> io::Result<()> {
    let dir = target.parent().unwrap_or(Path::new("."));
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let write = || -> io::Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        if let Ok(meta) = fs::metadata(target) {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()?;
        fs::rename(&tmp, target)
    };
    write().inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::DEFAULT_CONFIG;
    use super::*;

    /// A hand-edited file: comments everywhere, a legacy array, tables after the links.
    const COMMENTED: &str = r#"# My flick config.
hotkey = "cmd+Space"   # not the default

# Old-style links, still read.
[[quicklinks]]
name = "Old"   # kept from 2025
url = "https://old.example/{query}"

# Links.
[[quicklink.links]]
# work
name = "Work"
keyword = "w"   # short
url = "https://work.example"

[[quicklink.links]]
name = "Home"
url = "~/"

# Clipboard history.
[clip]
enabled = false
"#;

    fn entry(name: &str, url: &str) -> Entry {
        vec![("name", name.into()), ("url", url.into())]
    }

    fn append(text: &str, entry: Entry) -> Result<String, String> {
        edit_text(text, "quicklink", "links", &Edit::Append(entry))
    }

    fn replace(text: &str, name: &str, entry: Entry) -> Result<String, String> {
        edit_text(text, "quicklink", "links", &Edit::Replace { name: name.into(), entry })
    }

    fn remove(text: &str, name: &str) -> Result<String, String> {
        edit_text(text, "quicklink", "links", &Edit::Remove { name: name.into() })
    }

    /// Link names as the quicklink module sees them (table entries, then legacy ones).
    fn names(text: &str) -> Vec<String> {
        let c = parse(text).unwrap();
        let t = c.section("quicklink").unwrap().unwrap().get::<toml::Table>().unwrap();
        let links = t.get("links").and_then(toml::Value::as_array).cloned().unwrap_or_default();
        links.iter().map(|l| l["name"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn append_adds_only_the_new_entry() {
        let out = append(DEFAULT_CONFIG, entry("Docs", "https://docs.rs/{query}")).unwrap();
        let added = "\n[[quicklink.links]]\nname = \"Docs\"\nurl = \"https://docs.rs/{query}\"\n";
        assert_eq!(out, format!("{DEFAULT_CONFIG}{added}"));
        assert_eq!(names(&out).last().map(String::as_str), Some("Docs"));
    }

    #[test]
    fn append_goes_after_the_last_entry_of_its_array() {
        let out = append(COMMENTED, entry("New", "/")).unwrap();
        let expected = COMMENTED.replace(
            "url = \"~/\"\n",
            "url = \"~/\"\n\n[[quicklink.links]]\nname = \"New\"\nurl = \"/\"\n",
        );
        assert_eq!(out, expected);
        assert_eq!(names(&out), ["Work", "Home", "New", "Old"]);
    }

    #[test]
    fn append_uses_the_table_form_even_beside_a_legacy_array() {
        let legacy = "[[quicklinks]]\nname = \"Old\"\nurl = \"/\"\n";
        let out = append(legacy, entry("New", "/")).unwrap();
        assert_eq!(out, format!("{legacy}\n[[quicklink.links]]\nname = \"New\"\nurl = \"/\"\n"));
        assert_eq!(names(&out), ["New", "Old"]);
        let out = append("", entry("A", "/")).unwrap();
        assert_eq!(out, "[[quicklink.links]]\nname = \"A\"\nurl = \"/\"\n");
        let out = append("[quicklink]\nenabled = true\n", entry("A", "/")).unwrap();
        assert_eq!(names(&out), ["A"]);
    }

    #[test]
    fn append_extends_an_inline_array() {
        let inline = "[quicklink]\nlinks = [{ name = \"A\", url = \"/\" }]\n";
        let out = append(inline, entry("B", "~/")).unwrap();
        assert_eq!(
            out,
            "[quicklink]\nlinks = [{ name = \"A\", url = \"/\" }, { name = \"B\", url = \"~/\" }]\n"
        );
        assert!(append("quicklink = 1\n", entry("B", "/")).unwrap_err().starts_with("quicklink:"));
        assert!(append("[quicklink]\nlinks = 1\n", entry("B", "/")).is_err());
    }

    #[test]
    fn replace_keeps_comments_and_field_order() {
        let new = vec![
            ("name", "Job".to_string()),
            ("keyword", "j".into()),
            ("url", "https://job.example".into()),
            ("app", "Safari".into()),
        ];
        let out = replace(COMMENTED, "Work", new).unwrap();
        let expected = COMMENTED.replace(
            "name = \"Work\"\nkeyword = \"w\"   # short\nurl = \"https://work.example\"\n",
            "name = \"Job\"\nkeyword = \"j\"   # short\nurl = \"https://job.example\"\napp = \"Safari\"\n",
        );
        assert_eq!(out, expected);
        let out = replace(COMMENTED, "Work", entry("Work", "/")).unwrap();
        assert_eq!(
            out,
            COMMENTED.replace(
                "keyword = \"w\"   # short\nurl = \"https://work.example\"",
                "url = \"/\""
            )
        );
    }

    #[test]
    fn replace_and_remove_find_legacy_entries() {
        let out = replace(COMMENTED, "Old", entry("Older", "/")).unwrap();
        assert_eq!(
            out,
            COMMENTED.replace(
                "name = \"Old\"   # kept from 2025\nurl = \"https://old.example/{query}\"",
                "name = \"Older\"   # kept from 2025\nurl = \"/\""
            )
        );
        let out = remove(COMMENTED, "Old").unwrap();
        assert_eq!(names(&out), ["Work", "Home"]);
        assert!(!out.contains("Old"));
        assert!(
            out.starts_with("# My flick config.\nhotkey = \"cmd+Space\"   # not the default\n")
        );
        let inline =
            "quicklinks = [{ name = \"A\", url = \"/\" }, { name = \"B\", url = \"/\" }]\n";
        let out = remove(inline, "A").unwrap();
        assert_eq!(out, "quicklinks = [{ name = \"B\", url = \"/\" }]\n");
        let out = replace(inline, "B", entry("C", "~/")).unwrap();
        assert_eq!(out, inline.replace("\"B\", url = \"/\"", "\"C\", url = \"~/\""));
    }

    #[test]
    fn remove_drops_only_the_entry() {
        let out = remove(COMMENTED, "Home").unwrap();
        assert_eq!(
            out,
            COMMENTED.replace("\n[[quicklink.links]]\nname = \"Home\"\nurl = \"~/\"\n", "")
        );
        // The first entry's comment heads the array and stays for the next entry.
        let out = remove(DEFAULT_CONFIG, "Google").unwrap();
        let google = "[[quicklink.links]]\nname = \"Google\"\nkeyword = \"g\"\n\
                      url = \"https://www.google.com/search?q={query}\"\n\n";
        assert_eq!(out, DEFAULT_CONFIG.replace(google, ""));
        let mut all = DEFAULT_CONFIG.to_string();
        for name in ["Google", "GitHub Search", "YouTube", "Projects"] {
            all = remove(&all, name).unwrap();
        }
        assert!(names(&all).is_empty());
    }

    #[test]
    fn errors_leave_the_text_alone() {
        let err = remove(COMMENTED, "Nope").unwrap_err();
        assert_eq!(err, "no [[quicklink.links]] entry named \"Nope\"");
        assert!(replace(COMMENTED, "work", entry("x", "/")).is_err(), "names match exactly");
        let err = append("hotkey = [", entry("A", "/")).unwrap_err();
        assert!(err.starts_with("not edited, the file does not load"), "{err}");
        assert!(append("hotkey = 1", entry("A", "/")).is_err());
        // `desktop_toggle` is legacy for [desktop] hotkey; an array there cannot join it.
        let err = edit_text(
            "desktop_toggle = \"cmd+K\"\n",
            "desktop",
            "hotkey",
            &Edit::Append(entry("A", "/")),
        )
        .unwrap_err();
        assert!(err.starts_with("not edited, the result would not load"), "{err}");
    }

    /// A fresh directory under the system temp dir.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("flick-edit-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_through_a_symlink_atomically() {
        let dir = scratch("symlink");
        let target = dir.join("real.toml");
        let link = dir.join("config.toml");
        fs::write(&target, COMMENTED).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let edit = Edit::Remove { name: "Home".into() };
        edit_file(&link, "quicklink", "links", &edit).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), remove(COMMENTED, "Home").unwrap());
        assert_eq!(files(&dir), ["config.toml", "real.toml"]);

        let err = edit_file(&link, "quicklink", "links", &edit).unwrap_err();
        assert!(err.ends_with("no [[quicklink.links]] entry named \"Home\""), "{err}");
        fs::write(&target, "hotkey = [").unwrap();
        assert!(edit_file(&link, "quicklink", "links", &Edit::Append(entry("A", "/"))).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "hotkey = [");
        assert_eq!(files(&dir), ["config.toml", "real.toml"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_starts_from_the_default() {
        let dir = scratch("missing");
        let path = dir.join("sub/config.toml");
        edit_file(&path, "quicklink", "links", &Edit::Append(entry("A", "/"))).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            append(DEFAULT_CONFIG, entry("A", "/")).unwrap()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_keeps_permissions_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("perms");
        let path = dir.join("config.toml");
        fs::write(&path, "a").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        write_atomic(&path, "b").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "b");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // Renaming onto a directory fails; the temp file is removed.
        let sub = dir.join("sub");
        fs::create_dir(&sub).unwrap();
        assert!(write_atomic(&sub, "c").is_err());
        assert_eq!(files(&dir), ["config.toml", "sub"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
