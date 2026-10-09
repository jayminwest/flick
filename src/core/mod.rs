//! Core types every feature shares: items and their ids, outcomes, list views, forms,
//! item actions and confirmations, the `Module` contract and its registry, ranking, and the
//! key engine.
//! Plain Rust: no `crate::platform`, so everything here unit-tests without `AppKit`.

mod action;
mod confirm;
pub mod control;
mod event;
mod form;
mod item;
pub mod keys;
mod module;
mod rank;
mod registry;
mod routing;
pub mod track;
mod view;

pub use action::Action;
pub use confirm::Confirm;
pub use confirm::ConfirmRow;
pub use event::{Event, RecentPids};
pub use form::Field;
pub use form::Form;
pub use item::{Icon, Item, ItemId};
#[cfg(test)]
pub use module::test_cx;
pub use module::{Binding, Cx, Module, unknown_verb};
pub use rank::{Ranker, Usage, frecency};
pub use registry::Registry;
pub use view::{ListView, Outcome};
