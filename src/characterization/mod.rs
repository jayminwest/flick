//! Characterization tests: they pin behavior users and their data depend on, so the core
//! refactor (plan pl-c956) can move code and prove nothing changed.
//!
//! - item ids, which key the `usage` table in an existing flick.db
//! - legacy config.toml parsing
//! - the store schema, the 500-clip cap and clip dedupe
//! - window rect math
//! - launcher ranking order (fuzzy score plus frecency)
//! - window switcher order
//! - control socket reply shapes (text, `--json` values, errors)
//!
//! Each test reaches the code only through the `use` lines at the top of its file. When
//! code moves, fix those paths and nothing else; a changed assertion is a behavior change.
//! Run only these tests with `cargo test characterization`.

mod control_replies;
mod item_ids;
mod legacy_config;
mod legacy_view;
mod ranking;
mod store_fixture;
mod store_schema;
mod switcher_order;
mod window_rects;
