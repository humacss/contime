//! Shared robot simulation; environments supply elapsed time and execution.
pub mod config;
mod driver;
mod reference;
pub mod report;
pub mod scenario;
pub use config::Config;
pub use driver::Driver;
