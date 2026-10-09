//! Core types every feature shares: items and their ids, outcomes, list views, the
//! `Module` contract and its registry, and ranking. Plain Rust: no `crate::platform`, so
//! everything here unit-tests without `AppKit`.

mod event;
mod item;
mod module;
mod rank;
mod registry;
mod view;

pub use event::{Event, RecentPids};
pub use item::{Icon, Item, ItemId};
#[cfg(test)]
pub use module::test_cx;
pub use module::{Binding, Cx, Module};
pub use rank::{Ranker, Usage, frecency};
pub use registry::Registry;
pub use view::{ListView, Outcome};
