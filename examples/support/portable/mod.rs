//! Host-independent configuration and formatting, without terminal or scheduling code.
#[path = "../format.rs"]
mod format;
#[path = "../rate.rs"]
mod rate;
#[path = "../settings.rs"]
mod settings;
#[path = "../targets.rs"]
mod targets;
pub use format::{decimal, number};
pub use rate::EventRate;
pub use settings::Settings;
pub use targets::Average;

#[path = "../execution/mod.rs"]
pub mod execution;
#[path = "../movement/config.rs"]
pub mod movement_config;

#[path = "../movement/presentation.rs"]
pub mod movement_report;
