//! Optional field-based time access and function-based identity delegation.
mod creation;
mod events;
mod expand;
mod field;
mod identity;
mod metadata;
mod metadata_parameters;
mod metadata_paths;
mod options;
mod selector;
pub(crate) use expand::{event, snapshot};

pub(crate) use metadata_paths::module_type;
