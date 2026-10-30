//! Static module registration, dependency composition and aggregate lanes.
mod composition;
mod lanes;
mod metadata;
pub(crate) use composition::collect;
pub(crate) use metadata::{derive, register, Registration, State};
