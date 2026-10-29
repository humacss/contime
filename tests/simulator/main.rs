//! Event generation and optional backend/monitor integration through the facade.
#[cfg(feature = "monitor")]
mod admission;
mod generation;
#[cfg(any(feature = "runtime", feature = "manual"))]
mod pipeline;
mod stream;
#[path = "../support/mod.rs"]
mod support;
