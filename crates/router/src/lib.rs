//! Deterministic input-batch routing independent of ConTime orchestration.

mod hash;
mod route;
mod types;

pub use route::route;
pub use types::{InputBatch, RoutableInput, RoutedInput, RouterError, WorkerBatch};
