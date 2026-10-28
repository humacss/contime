//! Example-only execution adapters; load, completion feedback and UI stay shared.
use super::{Failure, Options};
use crate::definition::{Example, Time};
use contime::contime_monitor::{WorkerMonitorError, WorkerStatusSource};
use crossbeam_channel::{Receiver, Sender};

pub trait Backend<M: Example>: WorkerStatusSource<Time> {
    fn submit(&self, events: Vec<M::Event>) -> Result<(), Failure>;
    fn advance(&self, time: Time) -> Result<(), Failure>;
    fn query(&self, time: Time, count: usize, response: Sender<Vec<Box<M::State>>>) -> Result<(), Failure>;
    fn check_rejections(&self) -> Result<(), Failure>;
    /// Returns to the shared loop after one Manual operation; Runtime runs independently.
    fn process(&self) -> Result<bool, Failure>;
    /// Reserve control/query queue slots before admitting another simulator batch.
    fn can_submit(&self) -> bool;
    fn shutdown(self) -> Result<(), Failure>;
}

#[path = "../../environments/execution.rs"]
mod execution;
fn config(options: &Options) -> execution::Config {
    execution::Config { workers: options.workers, retention_ms: options.retention_ms, checkpoint_interval: options.checkpoint_interval }
}
#[cfg(feature = "runtime")]
pub use execution::Runtime;
#[cfg(feature = "runtime")]
pub fn runtime<M: Example>(options: &Options) -> Result<Runtime<M>, Failure>
where
    M::Event: Send + Sync,
    M::State: Send,
{
    execution::runtime::<M>(&config(options))
}
#[cfg(feature = "runtime")]
impl<M: Example> Backend<M> for Runtime<M>
where
    M::Event: Send + Sync,
    M::State: Send,
{
    fn submit(&self, events: Vec<M::Event>) -> Result<(), Failure> {
        self.apply(events).map_err(|e| format!("Submission failed: {e:?}").into())
    }
    fn advance(&self, time: Time) -> Result<(), Failure> {
        self.advance_to(time).map_err(|e| format!("Advancement failed: {e:?}").into())
    }
    fn query(&self, time: Time, count: usize, response: Sender<Vec<Box<M::State>>>) -> Result<(), Failure> {
        self.send_query_at(time, 1..=count as u128, response).map_err(|e| format!("Validation query failed: {e:?}").into())
    }
    fn check_rejections(&self) -> Result<(), Failure> {
        if let Some(rejection) = self.errors().try_iter().next() {
            return Err(format!("Rejected event: {rejection:?}").into());
        }
        Ok(())
    }
    fn process(&self) -> Result<bool, Failure> {
        Ok(false)
    }
    fn can_submit(&self) -> bool {
        true
    }
    fn shutdown(self) -> Result<(), Failure> {
        let shutdown = contime::ConTime::shutdown(self);
        if shutdown.workers.iter().any(|outcome| !matches!(outcome, contime::ThreadOutcome::Completed))
            || shutdown.routers.iter().any(|outcome| !matches!(outcome, contime::ThreadOutcome::Completed))
        {
            return Err(format!("Runtime shutdown failed: {shutdown:?}").into());
        }
        Ok(())
    }
}

#[cfg(feature = "manual")]
pub struct Manual<M: Example>(execution::Manual<M>);
#[cfg(feature = "manual")]
impl<M: Example> Manual<M> {
    pub fn new(options: &Options) -> Result<Self, Failure> {
        options.validate()?;
        Ok(Self(execution::Manual::new(&config(options))?))
    }
    pub fn queued(&self) -> usize {
        self.0.queued()
    }
}
#[cfg(feature = "manual")]
impl<M: Example> WorkerStatusSource<Time> for Manual<M> {
    fn worker_count(&self) -> usize {
        1
    }
    fn request_worker_status(&self) -> Result<Receiver<contime::contime_monitor::WorkerStatus<Time>>, WorkerMonitorError> {
        let (send, receive) = crossbeam_channel::bounded(1);
        let _ = send.try_send(self.0.inspect(|instance| instance.worker_status()));
        Ok(receive)
    }
}
#[cfg(feature = "manual")]
impl<M: Example> Backend<M> for Manual<M> {
    fn submit(&self, events: Vec<M::Event>) -> Result<(), Failure> {
        self.0.submit(events)
    }
    fn advance(&self, time: Time) -> Result<(), Failure> {
        self.0.advance(time)
    }
    fn query(&self, time: Time, count: usize, response: Sender<Vec<Box<M::State>>>) -> Result<(), Failure> {
        self.0.query(time, count, response)
    }
    fn check_rejections(&self) -> Result<(), Failure> {
        self.0.check_rejections()
    }
    fn process(&self) -> Result<bool, Failure> {
        self.0.process()
    }
    fn can_submit(&self) -> bool {
        self.0.can_submit()
    }
    fn shutdown(self) -> Result<(), Failure> {
        self.0.shutdown()
    }
}
