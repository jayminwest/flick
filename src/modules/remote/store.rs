//! The remote module's table, `remote_state`, and the only SQL that touches it. The network
//! access switch is user state, not config: off until the user turns it on.

use crate::core::store::Store;

/// Append only: released migrations are permanent.
pub const MIGRATIONS: &[&str] = &["CREATE TABLE remote_state (key TEXT PRIMARY KEY, value TEXT);"];

/// `remote_state` key of the switch ("1" on; anything else or missing is off).
const ON: &str = "on";

/// The network access switch on the shared store. A failed write leaves it as it was.
pub trait Switch {
    fn network_on(&self) -> bool;
    fn set_network_on(&self, on: bool);
}

impl Switch for Store {
    fn network_on(&self) -> bool {
        self.conn()
            .query_row("SELECT value FROM remote_state WHERE key = ?1", [ON], |r| {
                r.get::<_, Option<String>>(0)
            })
            .ok()
            .flatten()
            .as_deref()
            == Some("1")
    }

    fn set_network_on(&self, on: bool) {
        let _ = self.conn().execute(
            "INSERT INTO remote_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            [ON, if on { "1" } else { "0" }],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_is_off_until_set() {
        let s = Store::in_memory();
        s.migrate("remote", MIGRATIONS).unwrap();
        assert!(!s.network_on());
        s.set_network_on(true);
        assert!(s.network_on());
        s.set_network_on(false);
        assert!(!s.network_on());
    }

    #[test]
    fn a_missing_table_reads_as_off() {
        let s = Store::in_memory();
        s.set_network_on(true);
        assert!(!s.network_on());
    }
}
