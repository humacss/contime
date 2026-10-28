use std::fmt;
use std::ops::Deref;

use crate::contime_runtime::SharedEvent;

#[cfg(test)]
pub(crate) fn prepare_inputs<I: crate::contime_runtime::ThreadInput>(inputs: Vec<I>) -> Vec<SharedEvent<I>> {
    inputs.into_iter().map(SharedEvent::from).collect()
}

impl<I: crate::contime_runtime::ThreadInput> From<I> for SharedEvent<I> {
    fn from(input: I) -> Self {
        Self { inner: input.into(), observation: None }
    }
}

impl<I> Clone for SharedEvent<I>
where
    I: crate::contime_runtime::ThreadInput,
{
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), observation: self.observation.clone() }
    }
}

impl<I> Deref for SharedEvent<I>
where
    I: crate::contime_runtime::ThreadInput,
{
    type Target = I;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<I> fmt::Debug for SharedEvent<I>
where
    I: crate::contime_runtime::ThreadInput + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<I> AsRef<I> for SharedEvent<I>
where
    I: crate::contime_runtime::ThreadInput,
{
    fn as_ref(&self) -> &I {
        self
    }
}

impl<I> crate::contime_events::Event for SharedEvent<I>
where
    I: crate::contime_runtime::ThreadInput,
{
    type Time = I::Time;

    fn event_id(&self) -> u128 {
        self.deref().event_id()
    }

    fn time(&self) -> Self::Time {
        self.deref().time()
    }
}

impl<I> crate::contime_router::RoutableInput for SharedEvent<I>
where
    I: crate::contime_runtime::ThreadInput,
{
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        self.deref().snapshot_ids(emit);
    }
}
