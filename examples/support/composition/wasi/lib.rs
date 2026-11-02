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

#[path = "../../../support/uptime/mod.rs"]
pub mod uptime;
#[path = "../../../cases/service_uptime/mod.rs"]
pub mod uptime_example;
pub mod uptime_host;

#[path = "../../../support/accounts/mod.rs"]
pub mod accounts;
#[path = "../../../cases/account_balances/mod.rs"]
pub mod accounts_example;
pub mod accounts_host;

#[path = "../../../support/robotics/mod.rs"]
pub mod robotics;
#[path = "../../../cases/robotics/mod.rs"]
pub mod robotics_example;
pub mod robotics_host;
