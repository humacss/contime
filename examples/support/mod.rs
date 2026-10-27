//! Reusable example load, execution, and UI; domain definitions stay in their folders.
mod config;
mod rate;
mod settings;
#[allow(dead_code)] // Hosts select their own presentation adapter.
pub mod ui;
pub use config::Settings;
pub use rate::EventRate;
pub use ui::TerminalGuard;

mod format;

pub use format::{decimal, number};

pub mod definition;

#[allow(dead_code)] // Runtime and Manual hosts select different entry points.
pub mod execution;

mod targets;
pub use targets::Average;

#[allow(dead_code)] // Individual examples bypass the central launcher.
pub mod launcher;

#[cfg(all(any(feature = "runtime", feature = "manual"), feature = "monitor", feature = "counters"))]
pub mod monitored;

#[allow(dead_code)]
#[path = "monitored/metrics.rs"]
pub mod monitored_metrics;
#[allow(dead_code)]
#[path = "monitored/ui.rs"]
pub mod monitored_ui;
#[allow(dead_code)]
#[path = "monitored/wire.rs"]
pub mod monitored_wire;

#[path = "../environments/native/host.rs"]
#[allow(dead_code)] // Portable launcher paths do not run native application callbacks.
pub(crate) mod native;

#[allow(dead_code)] // Configuration is shared with the launcher without importing the movement host.
#[path = "movement/config.rs"]
pub(crate) mod movement_config;
#[allow(dead_code)] // Each host selects its applicable startup schema.
pub(crate) mod schema;

#[allow(dead_code)] // Direct movement startup is not used by every example.
#[path = "movement/configure.rs"]
mod movement_configure;

#[path = "movement/presentation.rs"]
pub mod movement_report;
