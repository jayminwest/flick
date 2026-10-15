//! ext: external programs add root items, views, actions and forms to the launcher (plan
//! pl-cbf8). This step holds the pure protocol layer only (`proto`); the module, its
//! transport and its registration land in later steps.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the ext module (flick-6db0) is the first caller")
)]

pub mod proto;
