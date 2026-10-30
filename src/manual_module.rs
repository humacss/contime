//! Module-selected construction over the existing caller-driven instance.
use crate::contime_core::{checkpoints::ApplyWrapper, module::Module};
use crate::manual::{ManualConfig, RawManualConTime};
use std::ops::{Deref, DerefMut};

/// Caller-driven instance whose module supplies its complete application types.
/// Construction starts no threads. Call `process()` from the host loop.
pub struct ManualConTime<M: Module, W = ()>
where
    W: ApplyWrapper<M::SnapshotLanes, M::EventLanes> + 'static,
{
    inner: RawManualConTime<M::EventLanes, M::SnapshotLanes, W>,
}
impl<M: Module, W> Deref for ManualConTime<M, W>
where
    W: ApplyWrapper<M::SnapshotLanes, M::EventLanes> + 'static,
{
    type Target = RawManualConTime<M::EventLanes, M::SnapshotLanes, W>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl<M: Module, W> DerefMut for ManualConTime<M, W>
where
    W: ApplyWrapper<M::SnapshotLanes, M::EventLanes> + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
impl<M: Module> ManualConTime<M> {
    /// Creates one caller-driven worker using ordinary application.
    pub fn new(config: ManualConfig) -> Self {
        Self { inner: RawManualConTime::new(config, ()) }
    }
    /// Creates the worker with custom application hooks.
    pub fn with_wrapper<W>(config: ManualConfig, wrapper: W) -> ManualConTime<M, W>
    where
        W: ApplyWrapper<M::SnapshotLanes, M::EventLanes> + 'static,
    {
        ManualConTime { inner: RawManualConTime::new(config, wrapper) }
    }
}
