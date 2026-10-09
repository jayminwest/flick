//! The flat config layout the characterization tests were written against, read back
//! through the real parser and modules: hotkeys from what each module binds, quicklinks
//! from the quicklink module. A legacy key means the same thing when this view matches.

use std::collections::BTreeMap;

use crate::config;
use crate::modules::{quicklinks, with_apps};

pub struct Config {
    pub hotkey: String,
    pub desktop_toggle: Option<String>,
    pub windows_hotkey: Option<String>,
    pub window_keys: BTreeMap<String, String>,
    pub quicklinks: Vec<quicklinks::Quicklink>,
}

impl Default for Config {
    fn default() -> Self {
        view(&config::Config::default()).expect("default config builds")
    }
}

pub fn parse(text: &str) -> Result<Config, String> {
    view(&config::parse(text)?)
}

fn view(c: &config::Config) -> Result<Config, String> {
    let bindings = with_apps(c, vec![])?.hotkeys();
    let spec =
        |module: &str| bindings.iter().find(|(m, _)| *m == module).map(|(_, b)| b.spec.clone());
    Ok(Config {
        hotkey: c.hotkey.clone(),
        desktop_toggle: spec("desktop"),
        windows_hotkey: spec("switcher"),
        window_keys: bindings
            .iter()
            .filter(|(m, _)| *m == "window")
            .map(|(_, b)| (b.key.clone().unwrap_or_else(|e| e), b.spec.clone()))
            .collect(),
        quicklinks: quicklinks::links(c)?,
    })
}
