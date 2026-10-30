//! Expansion of snapshot definitions and their event application adapters.
mod borrowed_events;
mod definition;
mod expand;
mod routing;
pub(crate) use expand::expand;
