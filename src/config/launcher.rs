//! `[launcher]`: settings of the launcher panel itself. The controller owns this table, like
//! the top-level `hotkey`; it is not a module, so it has no `enabled` key.

use serde::Deserialize;
use toml::{Table, Value};

/// The table's name in config.toml.
pub const TABLE: &str = "launcher";
/// The lowest opacity: below it, text over a busy desktop is hard to read.
pub const MIN_OPACITY: f64 = 0.6;
/// A little more see-through than the plain blur (1.0).
pub const DEFAULT_OPACITY: f64 = 0.9;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Launcher {
    /// The panel background's opacity: `MIN_OPACITY` (most see-through) to 1.0 (the full
    /// blur). Text and icons stay opaque. Values out of range are clamped.
    pub opacity: f64,
}

impl Default for Launcher {
    fn default() -> Self {
        Launcher { opacity: DEFAULT_OPACITY }
    }
}

impl Launcher {
    /// The `[launcher]` table of a whole config file, defaults when it has none. Errors on
    /// a value that is not a table, a wrong type or an unknown key.
    pub fn from_tables(tables: &Table) -> Result<Launcher, String> {
        let launcher: Launcher = match tables.get(TABLE) {
            None => Launcher::default(),
            Some(Value::Table(t)) => {
                Value::Table(t.clone()).try_into().map_err(|e| format!("[{TABLE}]: {e}"))?
            }
            Some(_) => return Err(format!("{TABLE}: expected a [{TABLE}] table")),
        };
        Ok(Launcher { opacity: clamp(launcher.opacity) })
    }
}

/// `opacity` in `MIN_OPACITY..=1.0`; NaN means the default.
fn clamp(opacity: f64) -> f64 {
    if opacity.is_nan() { DEFAULT_OPACITY } else { opacity.clamp(MIN_OPACITY, 1.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opacity(text: &str) -> Result<f64, String> {
        Launcher::from_tables(&toml::from_str(text).unwrap()).map(|l| l.opacity)
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_example_documents_the_table() {
        crate::config::example::assert_documents::<Launcher>(TABLE);
    }

    #[test]
    fn no_table_means_a_little_transparency() {
        assert!(near(opacity("").unwrap(), DEFAULT_OPACITY));
        assert!(near(opacity("[launcher]").unwrap(), DEFAULT_OPACITY));
    }

    const _: () = assert!(DEFAULT_OPACITY < 1.0 && DEFAULT_OPACITY > MIN_OPACITY);

    #[test]
    fn opacity_is_clamped_to_a_legible_range() {
        assert!(near(opacity("[launcher]\nopacity = 0.8").unwrap(), 0.8));
        assert!(near(opacity("[launcher]\nopacity = 1").unwrap(), 1.0));
        assert!(near(opacity("[launcher]\nopacity = 2.0").unwrap(), 1.0));
        assert!(near(opacity("[launcher]\nopacity = 0.1").unwrap(), MIN_OPACITY));
        assert!(near(opacity("[launcher]\nopacity = -1").unwrap(), MIN_OPACITY));
        assert!(near(opacity("[launcher]\nopacity = nan").unwrap(), DEFAULT_OPACITY));
    }

    #[test]
    fn bad_values_are_errors() {
        assert!(opacity("[launcher]\nopacity = \"high\"").unwrap_err().starts_with("[launcher]: "));
        assert!(opacity("[launcher]\nblur = 1").unwrap_err().starts_with("[launcher]: "));
        assert_eq!(opacity("launcher = 1").unwrap_err(), "launcher: expected a [launcher] table");
    }
}
