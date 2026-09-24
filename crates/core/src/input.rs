use std::fmt;
use std::ops::Deref;

use std::sync::Arc;

use crate::{Input, SharedEvent};

pub(crate) fn prepare_inputs<I: Input>(inputs: Vec<I>) -> Vec<SharedEvent<I>> {
    inputs.into_iter().map(|input| SharedEvent { inner: Arc::new(input) }).collect()
}

impl<I> Clone for SharedEvent<I>
where
    I: Input,
{
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<I> Deref for SharedEvent<I>
where
    I: Input,
{
    type Target = I;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<I> fmt::Debug for SharedEvent<I>
where
    I: Input + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<I> AsRef<I> for SharedEvent<I>
where
    I: Input,
{
    fn as_ref(&self) -> &I {
        self
    }
}

impl<I> contime_events::Event for SharedEvent<I>
where
    I: Input,
{
    type Time = I::Time;

    fn event_id(&self) -> u128 {
        self.deref().event_id()
    }

    fn time(&self) -> Self::Time {
        self.deref().time()
    }
}

impl<I> contime_router::RoutableInput for SharedEvent<I>
where
    I: Input,
{
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        self.deref().snapshot_ids(emit);
    }
}
