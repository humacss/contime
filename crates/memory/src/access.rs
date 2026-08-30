use std::fmt;
use std::ops::Deref;

use crate::types::{MemoryAccount, TrackedArc};

impl<T, M> Deref for TrackedArc<T, M>
where
    M: MemoryAccount,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner.value
    }
}

impl<T, M> AsRef<T> for TrackedArc<T, M>
where
    M: MemoryAccount,
{
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T, M> fmt::Debug for TrackedArc<T, M>
where
    T: fmt::Debug,
    M: MemoryAccount,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<T, M> PartialEq for TrackedArc<T, M>
where
    T: PartialEq,
    M: MemoryAccount,
{
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl<T, M> Eq for TrackedArc<T, M>
where
    T: Eq,
    M: MemoryAccount,
{
}
