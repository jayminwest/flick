//! Module `builtin`: Flick's own commands in root search. Ids are `builtin:<title>`.

use std::path::{Path, PathBuf};

use crate::config::{self, target};
use crate::core::{Cx, Icon, Item, ItemId, ListView, Module, Outcome};
use crate::platform::app;

pub struct Flick;

/// Title, SF Symbol, verb, keywords.
const BUILTINS: [(&str, &str, &str, &str); 6] = [
    ("Clipboard History", "doc.on.clipboard", "Open Command", "paste"),
    ("Switch Windows", "macwindow.on.rectangle", "Open Command", "focus alt tab"),
    ("Create Quicklink", "link.badge.plus", "Open Command", "add new link url bookmark"),
    ("Open Flick Config", "gearshape", "Run Command", "settings preferences"),
    ("Reload Flick Config", "arrow.clockwise", "Run Command", "refresh"),
    ("Quit Flick", "power", "Run Command", "exit"),
];

impl Module for Flick {
    fn id(&self) -> &'static str {
        "builtin"
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        BUILTINS
            .into_iter()
            .map(|(title, symbol, verb, keywords)| Item {
                subtitle: "Flick".into(),
                accessory: "Command".into(),
                keywords: vec![keywords.into()],
                ..Item::new(ItemId::new("builtin", title), title, verb, Icon::Symbol(symbol))
            })
            .collect()
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        match id.key() {
            "Clipboard History" => Outcome::Push(ListView::new("clip", "history")),
            "Switch Windows" => Outcome::Push(ListView::new("switcher", "windows")),
            // A request by name: `quicklink` builds and saves the form.
            "Create Quicklink" => Outcome::Form { module: "quicklink", name: "new".into() },
            "Open Flick Config" => {
                let base = config::config_path();
                let files = target::files_to_open(&base, config::host_name().as_deref());
                let _ = std::process::Command::new("open").arg("-t").args(&files).spawn();
                read_only_note(&files, Path::new(target::NIX_STORE)).map_or_else(
                    || {
                        cx.hide();
                        Outcome::Hide
                    },
                    |note| Outcome::Stay(Some(note)),
                )
            }
            "Reload Flick Config" => Outcome::ReloadConfig,
            "Quit Flick" => {
                app::quit();
                Outcome::Stay(None)
            }
            _ => Outcome::Stay(None),
        }
    }
}

/// Status for "Open Flick Config" when an opened file resolves into the Nix store at
/// `store`: the editor shows it read-only, so say where edits go instead.
fn read_only_note(files: &[PathBuf], store: &Path) -> Option<String> {
    let names: Vec<String> = files
        .iter()
        .filter(|f| target::in_store(f, store))
        .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    (!names.is_empty()).then(|| {
        let names = names.join(", ");
        format!("Read-only (Nix store): {names}. Edit the source in your Nix config, then rebuild")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_quicklink_asks_quicklink_for_its_form() {
        let id = ItemId::new("builtin", "Create Quicklink");
        let out = crate::core::test_cx("", |cx| Flick.activate(&id, cx));
        assert!(matches!(out, Outcome::Form { module: "quicklink", name } if name == "new"));
    }

    #[test]
    fn store_files_get_a_read_only_note() {
        let store = Path::new("/nix/store");
        let home = PathBuf::from("/Users/me/dotfiles/config.toml");
        let built = PathBuf::from("/nix/store/abc-hm/config.toml");
        let over = PathBuf::from("/nix/store/abc-hm/config.mbp.toml");
        assert_eq!(read_only_note(std::slice::from_ref(&home), store), None);
        assert_eq!(
            read_only_note(&[built, over], store).unwrap(),
            "Read-only (Nix store): config.toml, config.mbp.toml. Edit the source in your Nix \
             config, then rebuild"
        );
    }
}
