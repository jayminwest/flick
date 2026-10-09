//! Core types every feature shares: items and their ids, outcomes, list views, forms,
//! item actions and confirmations, the `Module` contract and its registry, and ranking.
//! Plain Rust: no `crate::platform`, so everything here unit-tests without `AppKit`.

#[cfg_attr(not(test), expect(dead_code, reason = "used from flick-1875 on"))]
mod action;
#[cfg_attr(not(test), expect(dead_code, reason = "used from flick-1875 on"))]
mod confirm;
pub mod control;
mod event;
#[cfg_attr(not(test), expect(dead_code, reason = "used from flick-1875 on"))]
mod form;
mod item;
mod module;
mod rank;
mod registry;
mod routing;
#[cfg_attr(not(test), expect(dead_code, reason = "used from flick-e2d5 and flick-feab on"))]
pub mod track;
mod view;

pub use action::Action;
pub use confirm::Confirm;
#[expect(unused_imports, reason = "used from flick-1875 on")]
pub use confirm::ConfirmRow;
pub use event::{Event, RecentPids};
#[cfg_attr(not(test), expect(unused_imports, reason = "used from flick-1875 on"))]
pub use form::Field;
pub use form::Form;
pub use item::{Icon, Item, ItemId};
#[cfg(test)]
pub use module::test_cx;
pub use module::{Binding, Cx, Module, unknown_verb};
pub use rank::{Ranker, Usage, frecency};
pub use registry::Registry;
pub use view::{ListView, Outcome};
