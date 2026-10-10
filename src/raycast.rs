//! Import quicklinks from Raycast's "Export Quicklinks" JSON.

use std::path::Path;

use serde::Deserialize;

use crate::config::{self, edit, edit::Edit};
use crate::modules::quicklinks::{self, Quicklink};

#[derive(Deserialize)]
struct RaycastLink {
    name: String,
    link: String,
}

/// Rewrite Raycast's `{argument …}` and `{query}` placeholders as `{query}`.
/// Returns the link and any placeholders Flick can't fill (e.g. `{clipboard}`).
fn convert_link(link: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut unsupported = Vec::new();
    let mut rest = link;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else { break };
        let inner = rest[start + 1..start + len].trim();
        out.push_str(&rest[..start]);
        if inner.starts_with("argument") || inner.eq_ignore_ascii_case("query") {
            out.push_str("{query}");
        } else {
            out.push_str(&rest[start..=start + len]);
            unsupported.push(format!("{{{inner}}}"));
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    (out, unsupported)
}

pub struct Import {
    pub added: Vec<Quicklink>,
    pub duplicates: usize,
    pub warnings: Vec<String>,
}

/// Convert `json`, skipping links whose name or URL is already in `existing` (or earlier in the file).
fn convert(json: &str, existing: &[Quicklink]) -> Result<Import, String> {
    let links: Vec<RaycastLink> =
        serde_json::from_str(json).map_err(|e| format!("not a Raycast quicklinks export: {e}"))?;
    let mut seen: Vec<(String, String)> =
        existing.iter().map(|q| (q.name.to_lowercase(), q.url.clone())).collect();
    let mut import = Import { added: vec![], duplicates: 0, warnings: vec![] };
    for link in links {
        let (url, unsupported) = convert_link(&link.link);
        let name = link.name.trim().to_string();
        if seen.iter().any(|(n, u)| *n == name.to_lowercase() || *u == url) {
            import.duplicates += 1;
            continue;
        }
        if !unsupported.is_empty() {
            import.warnings.push(format!("{name}: unsupported {}", unsupported.join(", ")));
        }
        seen.push((name.to_lowercase(), url.clone()));
        import.added.push(Quicklink { name, url, keyword: None, app: None });
    }
    Ok(import)
}

/// Add `links` to config file `path` as `[[quicklink.links]]` entries, after its last one.
/// Each write leaves a file that loads; a file whose links can't take more is left alone.
fn append(path: &Path, links: &[Quicklink]) -> Result<(), String> {
    for link in links {
        edit::edit_file(path, "quicklink", "links", &Edit::Append(link.entry()))
            .map_err(|e| format!("can't add the imported quicklinks: {e}"))?;
    }
    Ok(())
}

/// Append the quicklinks in Raycast export `path` to the config file.
pub fn import_file(path: &Path) -> Result<String, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let config = config::load()?;
    let import = convert(&json, &quicklinks::links(&config)?)?;

    // The overlay holds the links when it sets them; its list replaces config.toml's.
    let config_path = config::target::entries_file(
        &config::config_path(),
        config::host_name().as_deref(),
        "quicklink",
        "links",
    );
    append(&config_path, &import.added)?;

    let mut report = format!(
        "Added {} quicklinks to {} ({} duplicates skipped).",
        import.added.len(),
        config_path.display(),
        import.duplicates
    );
    report.extend(import.warnings.iter().map(|w| format!("\n  warning: {w}")));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_placeholders() {
        assert_eq!(convert_link("https://x.com/?q={argument}").0, "https://x.com/?q={query}");
        assert_eq!(
            convert_link(r#"https://x.com/{argument name="repo" default="a"}"#).0,
            "https://x.com/{query}"
        );
        assert_eq!(convert_link("https://x.com/?q={Query}").0, "https://x.com/?q={query}");
        let (url, unsupported) = convert_link("https://x.com/?q={clipboard}");
        assert_eq!(
            (url.as_str(), unsupported),
            ("https://x.com/?q={clipboard}", vec!["{clipboard}".to_string()])
        );
        assert_eq!(convert_link("http://localhost:3000/").0, "http://localhost:3000/");
    }

    #[test]
    fn skips_duplicates_by_name_or_url() {
        let existing = vec![Quicklink {
            name: "Docs".into(),
            url: "https://docs.rs".into(),
            keyword: None,
            app: None,
        }];
        let json = r#"[
            {"name": "docs", "link": "https://other.com"},
            {"name": "Rust Docs", "link": "https://docs.rs"},
            {"name": "Open WebUI", "link": "http://host:3000/", "icon": "link"},
            {"name": "Open-WebUI", "link": "http://host:3000/"},
            {"name": "Search", "link": "https://duckduckgo.com/?q={argument}"}
        ]"#;
        let import = convert(json, &existing).unwrap();
        let names: Vec<_> = import.added.iter().map(|q| q.name.as_str()).collect();
        assert_eq!(names, ["Open WebUI", "Search"]);
        assert_eq!(import.duplicates, 3);
        assert!(import.added[1].takes_query());
    }

    fn links() -> Vec<Quicklink> {
        vec![
            Quicklink {
                name: "Search".into(),
                url: "https://x.com/?q={query}".into(),
                keyword: None,
                app: None,
            },
            Quicklink {
                name: "Home".into(),
                url: "~/".into(),
                keyword: Some("h".into()),
                app: None,
            },
        ]
    }

    /// Config file `text` after importing `links()` into it, and the link names it then loads.
    fn import_into(name: &str, text: &str) -> (Result<(), String>, String, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("flick-raycast-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        let result = append(&path, &links());
        let out = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let names = config::parse(&out)
            .and_then(|c| quicklinks::links(&c))
            .map(|links| links.into_iter().map(|q| q.name).collect())
            .unwrap_or_default();
        (result, out, names)
    }

    #[test]
    fn appends_module_table_entries_that_load() {
        let (result, out, _) = import_into("empty", "");
        result.unwrap();
        assert_eq!(
            out,
            "[[quicklink.links]]\nname = \"Search\"\nurl = \"https://x.com/?q={query}\"\n\n\
             [[quicklink.links]]\nname = \"Home\"\nurl = \"~/\"\nkeyword = \"h\"\n"
        );
        // After the file's own links, ahead of any later table.
        let own = "[[quicklink.links]]\nname = \"Mine\"\nurl = \"/\"\n\n[clip]\nenabled = false\n";
        let (result, out, names) = import_into("own", own);
        result.unwrap();
        assert!(out.ends_with("keyword = \"h\"\n\n[clip]\nenabled = false\n"), "{out}");
        assert_eq!(names, ["Mine", "Search", "Home"]);
        // Legacy files still load, with the imported links after their own.
        let legacy = "hotkey = \"cmd+K\"\n[[quicklinks]]\nname = \"Old\"\nurl = \"/\"\n";
        let (result, out, names) = import_into("legacy", legacy);
        result.unwrap();
        assert!(out.starts_with(legacy));
        assert_eq!(names, ["Search", "Home", "Old"]);
        // An inline array takes the links inline.
        let (result, _, names) = import_into("inline", "[quicklink]\nlinks = []\n");
        result.unwrap();
        assert_eq!(names, ["Search", "Home"]);
    }

    #[test]
    fn a_file_whose_links_cant_take_more_is_left_alone() {
        for (name, text) in [("number", "[quicklink]\nlinks = 1\n"), ("broken", "hotkey = [")] {
            let (result, out, _) = import_into(name, text);
            let err = result.unwrap_err();
            assert!(err.starts_with("can't add the imported quicklinks: "), "{err}");
            assert_eq!(out, text);
        }
    }
}
