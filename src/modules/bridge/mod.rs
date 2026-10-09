//! bridge: hand a question and a snapshot of Flick's context to an agent session (claude or
//! pi) and keep its answer (flick-3aac). This step holds the pure core only: argv building
//! and output parsing (`agent`) and the system prompt (`brief`). The worker, the store and
//! the module land in later steps.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the bridge module (flick-3ffa) is the first caller")
)]

pub mod agent;
pub mod brief;
