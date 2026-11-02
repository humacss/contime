//! Static module registration, dependency composition and aggregate lanes.
mod composition;
mod definition;
mod lanes;
mod metadata;
pub(crate) use composition::collect;
pub(crate) use definition::derive;
pub(crate) use metadata::State;
