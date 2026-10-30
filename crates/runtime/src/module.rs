//! Module-aware construction delegating to the existing managed runtime.
use crate::contime_core::{checkpoints::ApplyWrapper, module::Module};
use crate::contime_runtime::{ConTime as RawConTime, ConTimeConfig, StartError, ThreadInput};
use std::ops::Deref;

/// A managed instance whose module selects its event, snapshot and time types.
///
/// `ConTime::<App>::start(config)` starts one composed application. Low-level
/// adapters can use `RawConTime<Input, Snapshot, Wrapper>` directly. Operations
/// are delegated to that same managed pipeline without another processing loop.
pub struct ModuleConTime<M: Module, W = ()>
where
    M::EventLanes: ThreadInput,
{
    inner: RawConTime<M::EventLanes, M::SnapshotLanes, W>,
}

impl<M: Module, W> Deref for ModuleConTime<M, W>
where
    M::EventLanes: ThreadInput,
{
    type Target = RawConTime<M::EventLanes, M::SnapshotLanes, W>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl<M: Module> ModuleConTime<M>
where
    M::EventLanes: ThreadInput,
    M::SnapshotLanes: Send,
{
    /// Starts the module with ordinary deterministic application.
    pub fn start(config: ConTimeConfig<M::Time>) -> Result<Self, StartError> {
        M::validate();
        Ok(Self { inner: RawConTime::start(config, ())? })
    }
    /// Starts the module with hooks around canonical timestamp batches.
    pub fn start_with_wrapper<W>(config: ConTimeConfig<M::Time>, wrapper: W) -> Result<ModuleConTime<M, W>, StartError>
    where
        W: ApplyWrapper<M::SnapshotLanes, M::EventLanes> + Clone + Send + 'static,
    {
        M::validate();
        Ok(ModuleConTime { inner: RawConTime::start(config, wrapper)? })
    }
    /// Starts with local processing observations independent from retention.
    pub fn start_with_progress<P: crate::contime_runtime::ProgressPolicy<M::Time>>(
        config: ConTimeConfig<M::Time>,
        policy: P,
    ) -> Result<Self, StartError> {
        M::validate();
        Ok(Self { inner: RawConTime::start_with_progress(config, (), policy)? })
    }
}
impl<M: Module, W> ModuleConTime<M, W>
where
    M::EventLanes: ThreadInput,
{
    /// Closes admission and joins the managed topology.
    pub fn shutdown(self) -> crate::contime_runtime::ShutdownReport<crate::contime_router::RouterError, std::convert::Infallible> {
        self.inner.shutdown()
    }
}
