//! Declarative startup definitions, independent of terminal state and execution.
mod browser;
pub(crate) mod common;
mod definition;
mod movement;
pub use browser::definitions as browser;
pub use common::{common, contime};
pub use definition::{Definition, ValueKind};
pub use movement::definitions as movement;
#[cfg(test)]
mod tests;
