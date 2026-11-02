//! Browser host for the same continuous workload as the terminal and WASI hosts.
#[cfg(target_arch = "wasm32")]
mod bindings;
#[cfg(target_arch = "wasm32")]
#[path = "../../../environments/browser/src/clock.rs"]
mod clock;
#[path = "../../../cases/counter/mod.rs"]
pub mod counter_example;
#[path = "../../../support/definition.rs"]
pub mod definition;
mod frame;
mod hash;
#[path = "../../../cases/chain_hash/mod.rs"]
pub mod hash_example;
#[path = "../../../environments/browser/src/scheduler.rs"]
mod scheduler;
mod session;
pub use support::monitored;
pub mod support;
pub use session::Session;

#[path = "../../../support/movement/portable.rs"]
pub mod movement;
#[path = "../../../cases/client_movement/mod.rs"]
pub mod movement_example;

mod movement_frame;
mod worker_report;

#[path = "../../../support/uptime/mod.rs"]
pub mod uptime;
#[path = "../../../cases/service_uptime/mod.rs"]
pub mod uptime_example;
mod uptime_frame;

#[path = "../../../support/accounts/mod.rs"]
pub mod accounts;
#[path = "../../../cases/account_balances/mod.rs"]
pub mod accounts_example;
mod accounts_frame;

#[path = "../../../cases/robotics/mod.rs"]
pub mod robotics_example;

#[path = "../../../support/robotics/mod.rs"]
pub mod robotics;
mod robotics_frame;
