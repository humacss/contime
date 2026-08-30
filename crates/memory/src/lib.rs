//! Isolated retained-allocation and pointer accounting.

mod budget;
mod types;

pub use types::{ConservativeSize, MemoryAccount, MemoryBudget, MemoryFull, MemoryKind};
