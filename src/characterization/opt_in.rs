//! KOTA and the local models are opt-in (flick-0c67): a Mac whose config never names them
//! (a work Mac) runs none of their code and shows nothing of them.

use crate::config::{Config, parse};
use crate::core::test_cx;
use crate::modules::{OPT_IN, opt_in_config, verbs, with_apps};

/// Words that would show KOTA or a local model on a Mac that never asked for them.
const WORDS: [&str; 6] = ["kota", "llm", "model", "dictat", "whisper", "parakeet"];

fn mentions(text: &str) -> Option<&'static str> {
    let text = text.to_lowercase();
    WORDS.into_iter().find(|w| text.contains(w))
}

#[test]
fn a_config_without_their_tables_registers_none_of_them() {
    let mut registry = with_apps(&Config::default(), vec![]).unwrap();
    assert_eq!(registry.off(), OPT_IN);
    let shown = test_cx("", |cx| [registry.items(cx), registry.direct(cx)].concat());
    for item in &shown {
        let text = format!(
            "{} {} {} {}",
            item.id.as_str(),
            item.title,
            item.subtitle,
            item.keywords.join(" ")
        );
        assert_eq!(mentions(&text), None, "root item {text}");
    }
    for (module, binding) in registry.hotkeys() {
        assert!(!OPT_IN.contains(&module), "{module} binds {}", binding.spec);
    }
    for line in verbs(&Config::default()) {
        assert!(!OPT_IN.iter().any(|id| line.starts_with(&format!("{id} "))), "{line}");
    }
    for id in OPT_IN {
        let reply = test_cx("", |cx| registry.command(&[id.into(), "status".into()], cx));
        assert_eq!(
            reply,
            Err(format!("{id} is off: turn it on in its [{id}] table in config.toml"))
        );
    }
}

#[test]
fn a_bare_header_or_enabled_true_turns_none_on() {
    let text = OPT_IN.map(|id| format!("[{id}]\nenabled = true")).join("\n");
    assert_eq!(with_apps(&parse(&text).unwrap(), vec![]).unwrap().off(), OPT_IN);
}

#[test]
fn one_key_turns_each_on() {
    let registry = with_apps(&parse(&opt_in_config()).unwrap(), vec![]).unwrap();
    assert!(registry.off().is_empty());
    let ids: Vec<&str> = registry.into_modules().iter().map(|m| m.id()).collect();
    assert!(OPT_IN.iter().all(|id| ids.contains(id)), "{ids:?}");
    // `enabled = false` still wins over a key.
    let off =
        format!("{}\n[llm]\nenabled = false", opt_in_config().replace("[llm]\nhistory = true", ""));
    assert_eq!(with_apps(&parse(&off).unwrap(), vec![]).unwrap().off(), ["llm"]);
}

#[test]
fn kota_names_no_machine_of_its_own() {
    // Turned on, KOTA needs this Mac's config to say where it runs.
    let err = with_apps(&parse("[kota]\nhotkey = \"cmd+K\"").unwrap(), vec![]).err().unwrap();
    assert_eq!(err, "[kota]: set machine, cwd, dash (docs/kota.md)");
}
