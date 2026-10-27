//! Owned managed ConTime instance and its public operations.
mod advance;
mod api;
mod apply;
mod idle;
mod listen;
mod query;
mod send;
mod shutdown;
mod start;
mod status;
pub use api::{ConTime, ConTimeConfig};
pub use idle::IdleError;
