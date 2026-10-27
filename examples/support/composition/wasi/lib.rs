//! WASI host composition: shared domains, load, and caller-driven execution.
#[path = "../../../cases/counter/mod.rs"]
pub mod counter_example;
#[path = "../../../support/definition.rs"]
pub mod definition;
#[path = "../../../cases/chain_hash/mod.rs"]
pub mod hash_example;
#[path = "../../../support/execution/portable.rs"]
mod portable;
#[path = "../../../support/load/verification.rs"]
mod verification;
pub use contime::manual::FrameBudget;
pub use counter_example::setup::CounterExample;
pub use hash_example::setup::HashExample;
pub use portable::Scenario;
pub use verification::Report;

pub mod support;

/// Standalone process termination; never used by browser sessions.
#[path = "../../../support/exit.rs"]
pub mod exit;

pub mod workload;
pub use support::monitored;

#[path = "../../../support/movement/portable.rs"]
pub mod movement;
#[path = "../../../cases/client_movement/mod.rs"]
pub mod movement_example;

#[path = "movement.rs"]
pub mod movement_host;

#[path = "../../../environments/wasi/src/input.rs"]
pub mod input;
