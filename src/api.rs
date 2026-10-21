use crate::{Input, RejectionReason, RouterMessage};
use crossbeam_channel::{Receiver, Sender};
use std::marker::PhantomData;
use std::sync::Arc;

/// Complete apply-and-query process configuration.
#[derive(Clone, Debug)]
pub struct ConTimeConfig<T> {
    pub router_count: usize,
    pub worker_count: usize,
    pub placement: contime_router::Placement,
    /// Minimum wall-clock spacing between safe-pruning measurement rounds.
    /// Zero starts the next required round immediately; safety fences still apply.
    pub pruning_interval: std::time::Duration,
    pub history_retention: T,
    pub worker: contime_worker::WorkerConfig,
    pub checkpoints: crate::checkpoints::CheckpointConfig,
}

/// A running apply-and-query pipeline.
pub struct ConTime<I, S, W>
where
    I: Input,
{
    pub(crate) runtime: contime_runtime::Runtime<RouterMessage<I, S>, contime_router::RouterError, std::convert::Infallible>,
    pub(crate) input: Sender<RouterMessage<I, S>>,
    pub(crate) coordinator: std::thread::JoinHandle<()>,
    pub(crate) errors: Receiver<contime_api::RejectionMessage<RejectionReason>>,
    pub(crate) error_sender: Sender<contime_api::RejectionMessage<RejectionReason>>,

    pub(crate) subscriptions: Vec<Sender<Sender<bool>>>,
    pub(crate) queues: Vec<Arc<dyn Fn() -> bool + Send + Sync>>,
    pub(crate) types: PhantomData<fn() -> (S, W)>,
}
