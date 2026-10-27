//! Host-independent movement state machine, input delivery and queried scene.
#![allow(dead_code, unused_imports)]
mod delivery;
mod driver;
mod queries;
mod reference;
pub(crate) mod render_state;
mod report;
pub use crate::movement_example::model::{Direction, MovementInput, SECOND};
pub use crate::support::movement_config::Config;
pub use driver::Driver;
pub use report::Report;

mod json;
pub use json::config;

mod direction;
pub use direction::parse as direction;
