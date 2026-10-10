//! Core types every feature shares: items and their ids, outcomes, list views, forms,
//! item actions and confirmations, the `Module` contract and its registry, ranking, the key
//! engine, the card model (`card`) and the markdown-lite parser (`markup`).
//! Plain Rust: no `crate::platform`, so everything here unit-tests without `AppKit`.

mod action;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the message module (flick-e3f2) and hud card_view (flick-aa43) are the first callers"
    )
)]
pub mod card;
mod confirm;
pub mod control;
mod event;
mod form;
mod item;
pub mod keys;
pub mod later;
pub mod markup;
mod module;
mod rank;
mod registry;
mod routing;
pub mod store;
pub mod track;
mod view;

pub use action::Action;
pub use confirm::Confirm;
pub use confirm::ConfirmRow;
pub use event::{Event, RecentPids};
pub use form::Field;
pub use form::Form;
pub use item::{Icon, Item, ItemId, Tab};
#[cfg(test)]
pub use module::test_cx;
pub use module::{Binding, Cx, Module, unknown_verb};
pub use rank::{Ranker, Usage, frecency};
pub use registry::Registry;
pub use view::{ListView, Outcome};
