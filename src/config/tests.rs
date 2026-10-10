//! Tests of `crate::config`: the default file, legacy keys and sections.

use std::collections::BTreeMap;

use super::*;

fn keys(c: &Config) -> BTreeMap<String, String> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct W {
        keys: BTreeMap<String, String>,
    }
    c.section("window").unwrap().unwrap().get::<W>().unwrap().keys
}

fn links(c: &Config) -> Vec<String> {
    #[derive(Deserialize)]
    struct L {
        name: String,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Q {
        links: Vec<L>,
    }
    let q: Q = c.section("quicklink").unwrap().unwrap().get().unwrap();
    q.links.into_iter().map(|l| l.name).collect()
}

#[test]
fn default_config_parses() {
    let c = Config::default();
    assert_eq!(c.hotkey, "alt+shift+Space");
    assert_eq!(links(&c), ["Google", "GitHub Search", "YouTube", "Projects"]);
    assert!(keys(&c).is_empty());
    assert!(c.section("keys").unwrap().unwrap().get::<Table>().unwrap().is_empty());
    assert!(c.section("activity").unwrap().unwrap().get::<Table>().unwrap().is_empty());
    assert!(c.section("task").unwrap().unwrap().get::<Table>().unwrap().is_empty());
}

/// The commented example in `DEFAULT_CONFIG` from `marker` to the next blank line, with
/// the comment marks removed.
fn uncommented(marker: &str) -> String {
    let start = DEFAULT_CONFIG.find(marker).unwrap();
    let end = start + DEFAULT_CONFIG[start..].find("\n\n").unwrap();
    DEFAULT_CONFIG[start..end]
        .lines()
        .map(|l| l.trim_start_matches('#').trim_start())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn default_config_keys_example_parses_uncommented() {
    let example = uncommented("# [keys]");
    let keys: Table = parse(&example).unwrap().section("keys").unwrap().unwrap().get().unwrap();
    assert_eq!(keys.get("hyper").and_then(Value::as_str), Some("caps_lock"));
    assert_eq!(keys.get("chord").and_then(Value::as_array).map(Vec::len), Some(1));
}

#[test]
fn default_config_activity_example_parses_uncommented() {
    let example = uncommented("# [activity]");
    let activity: Table =
        parse(&example).unwrap().section("activity").unwrap().unwrap().get().unwrap();
    assert_eq!(activity.get("titles").and_then(Value::as_bool), Some(false));
    for key in ["urls", "remote_titles", "remote_urls"] {
        assert_eq!(activity.get(key).and_then(Value::as_bool), Some(false), "{key}");
    }
    assert_eq!(activity.get("exclude").and_then(Value::as_array).map(Vec::len), Some(3));
    let rules = activity.get("rules").and_then(Value::as_array).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].get("category").and_then(Value::as_str), Some("code"));
}

#[test]
fn default_config_herdr_example_parses_uncommented() {
    let example = uncommented("# [herdr]");
    let herdr: Table = parse(&example).unwrap().section("herdr").unwrap().unwrap().get().unwrap();
    assert_eq!(herdr.get("remote_refresh_secs").and_then(Value::as_integer), Some(60));
    assert_eq!(herdr.get("notify").and_then(Value::as_array).map(Vec::len), Some(2));
    assert!(!DEFAULT_CONFIG.contains("\n[herdr]"), "the default leaves [herdr] commented");
}

#[test]
fn default_config_task_example_parses_uncommented() {
    let example = uncommented("# [task]");
    let task: Table = parse(&example).unwrap().section("task").unwrap().unwrap().get().unwrap();
    assert_eq!(task.get("hotkey").and_then(Value::as_str), Some("cmd+ctrl+alt+shift+KeyT"));
    assert!(!DEFAULT_CONFIG.contains("\n[task]"), "the default leaves [task] commented");
}

#[test]
fn default_config_feedback_example_parses_uncommented() {
    let example = uncommented("# [feedback]");
    let fb: Table = parse(&example).unwrap().section("feedback").unwrap().unwrap().get().unwrap();
    assert_eq!(fb.get("keyword").and_then(Value::as_str), Some("fb"));
    assert_eq!(fb.len(), 3);
    assert!(!DEFAULT_CONFIG.contains("\n[feedback]"), "the default leaves [feedback] commented");
}

#[test]
fn default_config_message_example_parses_uncommented() {
    let example = uncommented("# [message]");
    let config = parse(&example).unwrap();
    let section = config.section("message").unwrap().unwrap();
    let message: Table = section.get().unwrap();
    assert_eq!(message.get("name").and_then(Value::as_str), Some("KOTA"));
    assert_eq!(message.get("max_cards").and_then(Value::as_integer), Some(4));
    assert_eq!(message.len(), 9);
    assert!(!DEFAULT_CONFIG.contains("\n[message]"), "the default leaves [message] commented");
}

#[test]
fn default_config_capture_example_parses_uncommented() {
    let example = uncommented("# [capture]");
    let capture: Table =
        parse(&example).unwrap().section("capture").unwrap().unwrap().get().unwrap();
    assert_eq!(capture.get("dir").and_then(Value::as_str), Some("~/Pictures/Flick"));
    assert_eq!(capture.get("colors").and_then(Value::as_array).map(Vec::len), Some(5));
    assert_eq!(capture.get("halo_color").and_then(Value::as_str), Some("#ffcc00"));
    assert_eq!(capture.keys().filter(|k| k.ends_with("_hotkey")).count(), 6);
    assert!(!DEFAULT_CONFIG.contains("\n[capture]"), "the default leaves [capture] commented");
}

#[test]
fn default_config_remote_example_parses_uncommented() {
    let example = uncommented("# [remote]");
    let r: Table = parse(&example).unwrap().section("remote").unwrap().unwrap().get().unwrap();
    assert_eq!(r.get("port").and_then(Value::as_integer), Some(7419));
    assert!(!DEFAULT_CONFIG.contains("\n[remote]"), "the default leaves [remote] commented");
}

#[test]
fn legacy_keys_move_into_module_tables() {
    let c = parse("desktop_toggle = \"cmd+K\"\n[window_keys]\ncenter = \"ctrl+C\"").unwrap();
    let desktop = c.section("desktop").unwrap().unwrap();
    assert_eq!(desktop.get::<Table>().unwrap().get("hotkey").unwrap().as_str(), Some("cmd+K"));
    assert_eq!(keys(&c).get("center").map(String::as_str), Some("ctrl+C"));
    assert!(c.section("desktop_toggle").unwrap().unwrap().get::<Table>().unwrap().is_empty());
}

#[test]
fn legacy_and_module_keys_combine() {
    let text = "[[quicklinks]]\nname = \"Old\"\nurl = \"/\"\n\
                [[quicklink.links]]\nname = \"New\"\nurl = \"/\"\n\
                [window_keys]\ncenter = \"ctrl+C\"\n[window.keys]\nmaximize = \"ctrl+M\"";
    let c = parse(text).unwrap();
    assert_eq!(links(&c), ["New", "Old"]);
    assert_eq!(keys(&c).len(), 2);
    let err = parse("windows_hotkey = \"a\"\n[switcher]\nhotkey = \"b\"").unwrap_err();
    assert_eq!(err, "windows_hotkey and [switcher] hotkey are both set; keep one");
    let err = parse("[window_keys]\nc = \"a\"\n[window.keys]\nc = \"b\"").unwrap_err();
    assert!(err.starts_with("window_keys and [window] keys"), "{err}");
    assert!(parse("window = 1\n[window_keys]\nc = \"a\"").is_err());
}

#[test]
fn enabled_flag_turns_a_module_off() {
    let c = parse("window = 2\n[clip]\nenabled = false\n[app]\nenabled = true\nx = 1").unwrap();
    assert!(c.section("clip").unwrap().is_none());
    let app = c.section("app").unwrap().unwrap().get::<Table>().unwrap();
    assert_eq!(app.keys().collect::<Vec<_>>(), ["x"]);
    assert!(c.section("nothing").unwrap().is_some());
    assert_eq!(c.section("window").unwrap_err(), "window: expected a [window] table");
    let c = parse("[clip]\nenabled = \"no\"").unwrap();
    assert_eq!(c.section("clip").unwrap_err(), "[clip] enabled: expected true or false");
}

#[test]
fn section_errors_name_the_table() {
    #[derive(Deserialize, Debug)]
    struct S {
        #[expect(dead_code, reason = "only its absence is tested")]
        need: String,
    }
    let c = parse("[desktop]").unwrap();
    let err = c.section("desktop").unwrap().unwrap().get::<S>().unwrap_err();
    assert!(err.starts_with("[desktop]: missing field `need`"), "{err}");
}

#[test]
fn a_missing_file_gets_the_default_and_a_bad_one_names_its_path() {
    let dir = std::env::temp_dir().join(format!("flick-config-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("sub/config.toml");
    assert_eq!(load_from(&path, None).unwrap().hotkey, "alt+shift+Space");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
    std::fs::write(&path, "hotkey = 1").unwrap();
    let err = load_from(&path, None).unwrap_err();
    assert!(err.starts_with(&path.display().to_string()), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_config_path_is_flick_config_or_under_dot_config() {
    let path = config_path();
    match std::env::var_os("FLICK_CONFIG") {
        Some(env) => assert_eq!(path.as_os_str(), env),
        None => assert!(path.ends_with(".config/flick/config.toml"), "{}", path.display()),
    }
}

#[test]
fn load_merges_the_hosts_overlay_from_disk() {
    let dir = std::env::temp_dir().join(format!("flick-config-overlay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "hotkey = \"cmd+K\"\n[kota]\nhost = \"a\"").unwrap();
    std::fs::write(dir.join("config.mbp.toml"), "[kota]\nhost = \"b\"").unwrap();
    let host =
        |c: &Config| c.section("kota").unwrap().unwrap().get::<Table>().unwrap()["host"].clone();
    assert_eq!(host(&load_from(&path, Some("mbp")).unwrap()), Value::from("b"));
    assert_eq!(host(&load_from(&path, Some("other")).unwrap()), Value::from("a"));
    assert_eq!(host(&load_from(&path, None).unwrap()), Value::from("a"));
    let _ = std::fs::remove_dir_all(&dir);
}
