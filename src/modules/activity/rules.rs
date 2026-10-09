//! Table `[activity]`: privacy switches (`titles`, `urls`, `remote_titles`, `remote_urls`,
//! `exclude`), the recording `hotkey`, `merge_secs`, and `[[activity.rules]]`, which name a
//! project and category for spans at report time (never stored, so editing a rule
//! recategorizes history).

use regex_lite::{Regex, RegexBuilder};
use serde::Deserialize;

use crate::config::Section;
use crate::core::track::{MERGE_SECS, Subject};

/// Password managers: never recorded.
const EXCLUDE: [&str; 3] =
    ["com.agilebits.onepassword7", "com.1password.1password", "com.apple.keychainaccess"];

#[derive(Deserialize)]
#[serde(default)]
#[expect(clippy::struct_excessive_bools, reason = "one bool per [activity] privacy key")]
struct Settings {
    titles: bool,
    urls: bool,
    remote_titles: bool,
    remote_urls: bool,
    exclude: Vec<String>,
    hotkey: String,
    merge_secs: i64,
    rules: Vec<RuleSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            titles: false,
            urls: false,
            remote_titles: false,
            remote_urls: false,
            exclude: EXCLUDE.map(String::from).to_vec(),
            hotkey: String::new(),
            merge_secs: MERGE_SECS,
            rules: vec![],
        }
    }
}

#[derive(Deserialize)]
struct RuleSettings {
    app: Option<String>,
    title: Option<String>,
    project: Option<String>,
    category: Option<String>,
}

/// One `[[activity.rules]]` entry. `app` matches the bundle id or the app name, `title` the
/// window title (so a title rule never matches a span without one); both ignore case. A
/// rule without patterns matches everything.
struct Rule {
    app: Option<Regex>,
    title: Option<Regex>,
    project: Option<String>,
    category: Option<String>,
}

impl Rule {
    fn matches(&self, s: &Subject) -> bool {
        self.app.as_ref().is_none_or(|re| re.is_match(&s.app) || re.is_match(&s.name))
            && self.title.as_ref().is_none_or(|re| s.title.as_deref().is_some_and(|t| re.is_match(t)))
    }
}

/// The parsed `[activity]` table.
#[expect(clippy::struct_excessive_bools, reason = "one bool per [activity] privacy key")]
pub struct Config {
    /// Read and store window titles (off: Flick never reads them).
    pub titles: bool,
    /// Read and store a supported browser's front tab URL (off: Flick never asks).
    pub urls: bool,
    /// Remote callers (`Cx::remote`) with a grant see window titles (off: titles dropped).
    pub remote_titles: bool,
    /// Remote callers with a grant see URLs and domains (off: both dropped).
    pub remote_urls: bool,
    /// Lowercased bundle ids and app names never recorded.
    exclude: Vec<String>,
    pub hotkey: Option<String>,
    pub merge_secs: i64,
    rules: Vec<Rule>,
}

impl Default for Config {
    fn default() -> Self {
        let s = Settings::default();
        Config {
            titles: s.titles,
            urls: s.urls,
            remote_titles: s.remote_titles,
            remote_urls: s.remote_urls,
            exclude: s.exclude.iter().map(|e| e.to_lowercase()).collect(),
            hotkey: None,
            merge_secs: s.merge_secs,
            rules: vec![],
        }
    }
}

impl Config {
    pub fn parse(table: &Section) -> Result<Config, String> {
        compile(table.get()?).map_err(|e| format!("[activity]: {e}"))
    }

    /// The front app's window is followed for changes: titles or URLs are on.
    pub fn follows(&self) -> bool {
        self.titles || self.urls
    }

    /// App `app` (bundle id) named `name` is never recorded.
    pub fn excludes(&self, app: &str, name: &str) -> bool {
        let (app, name) = (app.to_lowercase(), name.to_lowercase());
        self.exclude.iter().any(|e| *e == app || *e == name)
    }

    /// The (category, project) of the first rule that matches `s`.
    pub fn classify(&self, s: &Subject) -> (Option<&str>, Option<&str>) {
        self.rules
            .iter()
            .find(|r| r.matches(s))
            .map_or((None, None), |r| (r.category.as_deref(), r.project.as_deref()))
    }
}

fn compile(s: Settings) -> Result<Config, String> {
    if s.merge_secs < 0 {
        return Err("merge_secs must not be negative".into());
    }
    let regex = |i: usize, field: &str, p: Option<String>| {
        p.map(|p| {
            RegexBuilder::new(&p)
                .case_insensitive(true)
                .build()
                .map_err(|e| format!("rules[{i}].{field}: {e}"))
        })
        .transpose()
    };
    let rules = s
        .rules
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            Ok(Rule {
                app: regex(i, "app", r.app)?,
                title: regex(i, "title", r.title)?,
                project: r.project,
                category: r.category,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(Config {
        titles: s.titles,
        urls: s.urls,
        remote_titles: s.remote_titles,
        remote_urls: s.remote_urls,
        exclude: s.exclude.iter().map(|e| e.to_lowercase()).collect(),
        hotkey: Some(s.hotkey).filter(|h| !h.trim().is_empty()),
        merge_secs: s.merge_secs,
        rules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    fn config(text: &str) -> Result<Config, String> {
        Config::parse(&parse(text)?.section("activity")?.unwrap())
    }

    #[test]
    fn defaults_are_private() {
        let c = config("").unwrap();
        assert!(!c.titles, "titles stay off unless configured");
        assert!(!c.urls && !c.remote_urls && !c.follows(), "URLs stay off unless configured");
        assert!(c.excludes("com.agilebits.onepassword7", "1Password 7"));
        assert!(c.excludes("COM.APPLE.KEYCHAINACCESS", "Keychain Access"));
        assert!(!c.excludes("com.apple.Safari", "Safari"));
        assert_eq!((c.hotkey, c.merge_secs), (None, MERGE_SECS));
        assert!(!Config::default().titles);
    }

    #[test]
    fn settings_parse_and_fail_with_the_table_name() {
        let c = config(
            "[activity]\ntitles = true\nexclude = [\"Mail\"]\nhotkey = \"cmd+F9\"\nmerge_secs = 0",
        )
        .unwrap();
        assert!(c.titles && c.excludes("x", "mail") && !c.excludes("com.agilebits.onepassword7", ""));
        let u = config("[activity]\nurls = true\nremote_urls = true").unwrap();
        assert!(u.urls && u.remote_urls && !u.titles && u.follows());
        assert_eq!((c.hotkey.as_deref(), c.merge_secs), (Some("cmd+F9"), 0));
        let err = config("[activity]\nmerge_secs = -1").err().unwrap();
        assert_eq!(err, "[activity]: merge_secs must not be negative");
        let err = config("[[activity.rules]]\napp = \"(\"").err().unwrap();
        assert!(err.starts_with("[activity]: rules[0].app: "), "{err}");
        let err = config("[[activity.rules]]\ntitle = \"[\"").err().unwrap();
        assert!(err.starts_with("[activity]: rules[0].title: "), "{err}");
        assert!(config("[activity]\ntitles = 1").err().unwrap().starts_with("[activity]: "));
    }

    #[test]
    fn first_matching_rule_wins() {
        let c = config(
            "[[activity.rules]]\napp = \"wezterm|zed\"\ntitle = \"flick\"\nproject = \"flick\"\ncategory = \"code\"\n\
             [[activity.rules]]\napp = \"WEZTERM\"\ncategory = \"terminal\"\n\
             [[activity.rules]]\nproject = \"misc\"",
        )
        .unwrap();
        let s = |app: &str, title: Option<&str>| Subject::new(app, "Name", title, None);
        assert_eq!(c.classify(&s("dev.zed.Zed", Some("~/flick"))), (Some("code"), Some("flick")));
        // A title rule never matches a span without a title.
        assert_eq!(c.classify(&s("com.github.wez.wezterm", None)), (Some("terminal"), None));
        assert_eq!(c.classify(&s("com.apple.Safari", None)), (None, Some("misc")));
        assert_eq!(config("").unwrap().classify(&s("a", None)), (None, None));
        // The app pattern also matches the display name.
        assert_eq!(c.classify(&Subject::new("x", "WezTerm", None, None)).0, Some("terminal"));
    }

    #[test]
    fn the_example_config_lists_every_key() {
        crate::config::example::assert_documents::<Settings>("activity");
        crate::config::example::assert_documents::<RuleSettings>("activity.rules");
    }
}
