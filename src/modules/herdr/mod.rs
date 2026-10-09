//! herdr: monitor coding agents across machines and jump to them (flick-3cc9). So far the
//! pure fleet model and the transports (local socket, remote `herdr --machine`); the module
//! itself lands in flick-6d92.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the herdr module (flick-6d92) is the first caller")
)]

pub mod local;
pub mod model;
pub mod remote;
