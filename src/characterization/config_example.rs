//! `config.example.toml` (`flick config example`) lists every module's table, and with every
//! setting uncommented it is a config that every module accepts. Each module's own tests
//! check that its table lists exactly the keys it reads (`config::example::assert_documents`).

use crate::config::example::{EXAMPLE, uncommented};
use crate::config::parse;
use crate::modules::{IDS, with_apps};

#[test]
fn the_example_is_inert_until_uncommented() {
    let config = parse(EXAMPLE).unwrap();
    assert_eq!(config.hotkey, "alt+shift+Space");
    for id in IDS {
        assert!(config.section(id).unwrap().unwrap().get::<toml::Table>().unwrap().is_empty());
    }
}

#[test]
fn the_example_has_a_table_for_every_module_and_the_launcher() {
    let text = uncommented(EXAMPLE);
    let top: toml::Table = toml::from_str(&text).unwrap();
    let mut keys: Vec<&str> = top.keys().map(String::as_str).collect();
    let mut want: Vec<&str> = IDS.iter().copied().chain(["hotkey", "launcher"]).collect();
    keys.sort_unstable();
    want.sort_unstable();
    assert_eq!(keys, want);
}

#[test]
fn every_module_accepts_the_uncommented_example() {
    let config = parse(&uncommented(EXAMPLE)).unwrap();
    assert_eq!(config.hotkey, "alt+shift+Space");
    let registry = with_apps(&config, vec![]).unwrap();
    for (module, binding) in registry.hotkeys() {
        assert!(binding.key.is_ok(), "{module} {}: {:?}", binding.spec, binding.key);
    }
    let ids: Vec<&str> = registry.into_modules().iter().map(|m| m.id()).collect();
    assert_eq!(ids, IDS);
}
