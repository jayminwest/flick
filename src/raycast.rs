//! Import quicklinks from Raycast's "Export Quicklinks" JSON.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config;
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
        import.added.push(Quicklink { name, url, keyword: None });
    }
    Ok(import)
}

/// `[[quicklink.links]]` entries, as appended to config.toml.
#[derive(Serialize)]
struct Appended<'a> {
    quicklink: Links<'a>,
}

#[derive(Serialize)]
struct Links<'a> {
    links: &'a [Quicklink],
}

/// The text to append to config file `text` for `links`, checked to leave a loadable file
/// (an inline `links = [...]` array can't take `[[quicklink.links]]` after it).
fn appendix(text: &str, links: &[Quicklink]) -> Result<String, String> {
    let toml =
        toml::to_string(&Appended { quicklink: Links { links } }).map_err(|e| e.to_string())?;
    let extra = format!("\n# Imported from Raycast\n{toml}");
    config::parse(&format!("{text}{extra}"))
        .and_then(|c| quicklinks::links(&c))
        .map_err(|e| format!("can't add the imported quicklinks: {e}"))?;
    Ok(extra)
}

/// Append the quicklinks in Raycast export `path` to the config file.
pub fn import_file(path: &Path) -> Result<String, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let config = config::load()?;
    let import = convert(&json, &quicklinks::links(&config)?)?;

    let config_path = config::config_path();
    if !import.added.is_empty() {
        let text = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("{}: {e}", config_path.display()))?;
        let extra = appendix(&text, &import.added)
            .map_err(|e| format!("{}: {e}", config_path.display()))?;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&config_path)
            .map_err(|e| format!("{}: {e}", config_path.display()))?;
        file.write_all(extra.as_bytes()).map_err(|e| e.to_string())?;
    }

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
        let existing =
            vec![Quicklink { name: "Docs".into(), url: "https://docs.rs".into(), keyword: None }];
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

    #[test]
    fn appends_module_table_entries_that_load() {
        let added = vec![
            Quicklink {
                name: "Search".into(),
                url: "https://x.com/?q={query}".into(),
                keyword: None,
            },
            Quicklink { name: "Home".into(), url: "~/".into(), keyword: Some("h".into()) },
        ];
        assert_eq!(
            appendix("", &added).unwrap(),
            "\n# Imported from Raycast\n\
             [[quicklink.links]]\nname = \"Search\"\nurl = \"https://x.com/?q={query}\"\n\n\
             [[quicklink.links]]\nname = \"Home\"\nurl = \"~/\"\nkeyword = \"h\"\n"
        );
        // Legacy files still load, with the imported links after their own.
        let legacy = "hotkey = \"cmd+K\"\n[[quicklinks]]\nname = \"Old\"\nurl = \"/\"\n";
        let config = config::parse(&format!("{legacy}{}", appendix(legacy, &added).unwrap()));
        let names: Vec<_> =
            quicklinks::links(&config.unwrap()).unwrap().into_iter().map(|q| q.name).collect();
        assert_eq!(names, ["Search", "Home", "Old"]);
        // A file whose links can't take more is left alone.
        let inline = "[quicklink]\nlinks = []\n";
        assert!(appendix(inline, &added).unwrap_err().starts_with("can't add the imported"));
    }
}
