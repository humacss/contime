//! Example-owned worker presentation using public monitor readings and rates.
mod render;
mod view;
pub use view::{Scroll, WorkerView};
#[cfg(test)]
mod tests;
