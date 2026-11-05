//! tile: Flick's tiling window manager, replacing `AeroSpace` (plan pl-41fb). This step holds
//! pure core pieces only: the split tree of a workspace (`tree`) and the frames it gives its
//! windows (`layout`), workspaces and screen pins (`workspace`), app pins and float rules
//! (`rules`) and directional neighbours (`nav`). The module itself, its wiring and the
//! platform calls land with flick-ac1d.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the tile module (flick-ac1d) is the first caller")
)]

pub mod layout;
pub mod nav;
pub mod rules;
pub mod tree;
pub mod workspace;
