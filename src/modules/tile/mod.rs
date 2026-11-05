//! tile: Flick's tiling window manager (plan pl-41fb). This step holds pure core pieces
//! only: workspaces and screen pins (`workspace`), app pins and float rules (`rules`) and
//! directional neighbours (`nav`). The module itself lands with flick-ac1d.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the tile module (flick-ac1d) is the first caller")
)]

pub mod nav;
pub mod rules;
pub mod workspace;
