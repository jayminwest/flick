//! herdr: monitor coding agents across machines and jump to them (flick-3cc9). This step
//! holds the pure fleet model only; the transport and the module land in later steps.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the herdr module (flick-6d92) is the first caller")
)]

pub mod model;
