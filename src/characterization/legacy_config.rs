//! Existing config.toml files must keep parsing with the same meaning.

use crate::config::{Config, parse};
use crate::modules::windows::action::WindowAction;

/// Every key Flick has read so far, in the flat top-level layout.
const LEGACY: &str = r#"
hotkey = "cmd+Space"
desktop_toggle = "cmd+Backquote"
windows_hotkey = "alt+Tab"

[window_keys]
left-half = "ctrl+alt+ArrowLeft"
right-half = "ctrl+alt+ArrowRight"
maximize = "ctrl+alt+Enter"
next-display = "ctrl+alt+cmd+ArrowRight"

[[quicklinks]]
name = "Google"
keyword = "g"
url = "https://www.google.com/search?q={query}"

[[quicklinks]]
name = "Projects"
url = "~/Projects"
"#;

#[test]
fn legacy_keys_parse() {
    let c = parse(LEGACY).unwrap();
    assert_eq!(c.hotkey, "cmd+Space");
    assert_eq!(c.desktop_toggle.as_deref(), Some("cmd+Backquote"));
    assert_eq!(c.windows_hotkey.as_deref(), Some("alt+Tab"));
    let keys: Vec<(&str, &str)> =
        c.window_keys.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(
        keys,
        [
            ("left-half", "ctrl+alt+ArrowLeft"),
            ("maximize", "ctrl+alt+Enter"),
            ("next-display", "ctrl+alt+cmd+ArrowRight"),
            ("right-half", "ctrl+alt+ArrowRight"),
        ]
    );
    let links: Vec<(&str, &str, Option<&str>)> = c
        .quicklinks
        .iter()
        .map(|q| (q.name.as_str(), q.url.as_str(), q.keyword.as_deref()))
        .collect();
    assert_eq!(
        links,
        [
            ("Google", "https://www.google.com/search?q={query}", Some("g")),
            ("Projects", "~/Projects", None),
        ]
    );
}

#[test]
fn window_key_names_are_action_slugs() {
    let c = parse(LEGACY).unwrap();
    for name in c.window_keys.keys() {
        assert!(WindowAction::from_slug(name).is_some(), "{name}");
    }
    let slugs: Vec<String> = WindowAction::ALL.iter().map(|a| a.slug()).collect();
    assert_eq!(
        slugs,
        [
            "left-half",
            "right-half",
            "top-half",
            "bottom-half",
            "top-left-quarter",
            "top-right-quarter",
            "bottom-left-quarter",
            "bottom-right-quarter",
            "first-third",
            "center-third",
            "last-third",
            "first-two-thirds",
            "last-two-thirds",
            "maximize",
            "almost-maximize",
            "center",
            "next-display",
            "previous-display",
            "minimize",
            "hide",
        ]
    );
    for a in WindowAction::ALL {
        assert_eq!(WindowAction::from_slug(&a.slug()), Some(a));
    }
    assert_eq!(WindowAction::from_slug("left_half"), None);
}

#[test]
fn missing_keys_take_defaults() {
    let c = parse("").unwrap();
    assert_eq!(c.hotkey, "alt+shift+Space");
    assert_eq!((c.desktop_toggle, c.windows_hotkey), (None, None));
    assert!(c.window_keys.is_empty());
    assert!(c.quicklinks.is_empty());
}

#[test]
fn unknown_keys_and_tables_are_ignored() {
    let c = parse("hotkey = \"alt+Space\"\nfuture = 1\n[clipboard]\nenabled = false\n").unwrap();
    assert_eq!(c.hotkey, "alt+Space");
}

#[test]
fn malformed_files_are_errors() {
    assert!(parse("hotkey = 1").is_err());
    assert!(parse("[[quicklinks]]\nname = \"No URL\"").is_err());
    assert!(parse("hotkey = ").is_err());
}

#[test]
fn default_config_matches_first_run_file() {
    let c = Config::default();
    assert_eq!(c.hotkey, "alt+shift+Space");
    assert_eq!((c.desktop_toggle, c.windows_hotkey), (None, None));
    assert!(c.window_keys.is_empty());
    let links: Vec<(&str, Option<&str>, bool)> = c
        .quicklinks
        .iter()
        .map(|q| (q.name.as_str(), q.keyword.as_deref(), q.takes_query()))
        .collect();
    assert_eq!(
        links,
        [
            ("Google", Some("g"), true),
            ("GitHub Search", Some("gh"), true),
            ("YouTube", Some("yt"), true),
            ("Projects", None, false),
        ]
    );
}
