//! A complete caller-driven instance using the ordinary ConTime application contracts.
//!
//! Create an instance, enqueue operations, then call [`ManualConTime::process`](crate::manual::ManualConTime::process)
//! from the host loop. Query callbacks run during processing; they may capture `Rc`
//! state. No callback is invoked for a request discarded before processing. Manual
//! retention is explicit through [`ManualConTime::prune_before`](crate::manual::ManualConTime::prune_before).
//!
//! Subscriptions are not exposed by this convenience API. Advanced applications
//! can use [`WorkerDriver`](crate::manual::WorkerDriver) with their own worker message and storage contracts.
#![doc = concat!("\n# Counter example\n\n```no_run\n", include_str!("../examples/cases/counter/manual.rs"), "\n```\n")]
use contime_api::manual::Message;
pub use contime_core::InsertionRejection;
use contime_core::{
    checkpoints::{ApplyEvents, ApplyWrapper, CheckpointConfig, Snapshot},
    CheckpointStorage, CheckpointStorageConfig, Input, SharedEvent,
};
pub use contime_manual::{FrameBudget, FrameReport, ManualConTime as WorkerDriver, ProcessError, StopReason};
use contime_worker::RoutedInput;
use std::{cell::RefCell, num::NonZeroUsize, rc::Rc, time::Duration};

/// Queue capacity and checkpoint spacing for one manually driven worker.
pub struct ManualConfig {
    /// Maximum queued messages. A submission batch counts as one message;
    /// this does not bound retained history or events within one batch.
    pub queue_capacity: NonZeroUsize,
    /// Checkpoint spacing shared with the managed runtime.
    pub checkpoints: CheckpointConfig,
}
/// The bounded message queue could not accept another operation.
/// The operation was not enqueued; no callback will run for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueFull;
impl std::fmt::Display for QueueFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("manual ConTime queue is full")
    }
}
impl std::error::Error for QueueFull {}
type Request<I, S> = Message<SharedEvent<I>, S, <I as crate::checkpoints::Event>::Time, InsertionRejection>;

/// One caller-driven worker with Core's canonical history/storage adapters.
///
/// Uses the same `Input`, `Snapshot` and `ApplyEvents` contracts as the managed
/// runtime without imposing thread-safety bounds. Queue operations return
/// immediately. Processing and query callbacks execute on the calling thread.
/// A single complete operation may exceed the soft processing budget.
pub struct ManualConTime<I, S, W = ()>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I> + 'static,
    W: ApplyWrapper<S, I> + 'static,
{
    driver: WorkerDriver<Request<I, S>, CheckpointStorage<I, S, W>>,
    rejections: Rc<RefCell<Vec<InsertionRejection>>>,
}
impl<I, S, W> ManualConTime<I, S, W>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I> + 'static,
    W: ApplyWrapper<S, I> + 'static,
{
    /// Create one worker; no background work or threads are started.
    pub fn new(config: ManualConfig, wrapper: W) -> Self {
        Self {
            driver: WorkerDriver::new(config.queue_capacity, CheckpointStorageConfig { checkpoints: config.checkpoints }, wrapper),
            rejections: Rc::default(),
        }
    }
    fn enqueue(&mut self, message: Request<I, S>) -> Result<(), QueueFull> {
        self.driver.enqueue(message).map_err(|_| QueueFull)
    }
    /// Queue a batch for insertion. Every event supplies its destination histories;
    /// each route shares immutable ownership of that event. Success acknowledges
    /// queueing only; collect insertion rejections through [`Self::take_rejections`].
    /// A full queue discards this submission without inserting any of its routes.
    pub fn apply(&mut self, inputs: impl IntoIterator<Item = I>) -> Result<(), QueueFull> {
        let mut routes = Vec::new();
        for input in inputs {
            let input = SharedEvent::from(input);
            input.snapshot_ids(&mut |snapshot_id| routes.push(RoutedInput { snapshot_id, input: input.clone() }));
        }
        let errors = self.rejections.clone();
        self.enqueue(Message::apply(routes, move |rejected| errors.borrow_mut().extend(rejected)))
    }
    /// Queue a query through an inclusive timestamp. The callback runs once during
    /// processing, including for an empty result. Queries do not advance live replay
    /// or publish replay effects. Results need not be sorted by history ID.
    pub fn query_at(
        &mut self,
        time: I::Time,
        ids: impl IntoIterator<Item = u128>,
        callback: impl FnOnce(Vec<Box<S>>) + 'static,
    ) -> Result<(), QueueFull> {
        self.enqueue(Message::query_at(time, ids.into_iter().collect(), callback))
    }
    /// Queue a retained-event query in canonical order over `[from, to)`.
    /// Missing/empty histories invoke the callback with an empty vector.
    pub fn query_events_between(
        &mut self,
        id: u128,
        from: I::Time,
        to: I::Time,
        callback: impl FnOnce(Vec<SharedEvent<I>>) + 'static,
    ) -> Result<(), QueueFull> {
        self.enqueue(Message::query_events(id, from, to, callback))
    }
    /// Queue a monotonically increasing live processing target.
    /// Acceptance is separate from completing live replay.
    pub fn advance_to(&mut self, time: I::Time) -> Result<(), QueueFull> {
        self.enqueue(Message::advance(time))
    }
    /// Queue explicit retention. Preceding replay finishes before pruning; later
    /// messages wait for the prune pass. A horizon beyond the processing target
    /// returns [`ProcessError`] when processing reaches this request.
    pub fn prune_before(&mut self, time: I::Time) -> Result<(), QueueFull> {
        self.enqueue(Message::prune(time))
    }
    /// Perform one complete processing step, then return control to the host.
    /// Returns `true` if progress was made, or `false` when nothing is currently
    /// runnable. A step handles one message, replay/pruning call, or barrier
    /// transition, and may apply several events. Never sleeps or waits for work.
    pub fn process(&mut self) -> Result<bool, ProcessError> {
        self.driver.process()
    }
    /// Repeatedly process work up to a maximum time/work allowance.
    /// Uses the host's monotonic clock and returns early when no work is ready.
    /// Never sleeps or waits to fill the allowance. The time limit is checked
    /// between complete steps, so one step may overrun this soft maximum.
    /// Zero duration or work limit pauses processing. Callbacks must not block on
    /// this instance. Panics propagate; discard the instance after a caught panic.
    pub fn process_with_clock(&mut self, budget: FrameBudget, clock: impl FnMut() -> Duration) -> Result<FrameReport, ProcessError> {
        self.driver.process_with_clock(budget, clock)
    }
    /// Inspect this instance's one worker (ID zero) without processing work.
    /// Reports existing orchestration gauges and optional `counters` metrics.
    /// This scans history slots and the due schedule prefix; it neither queries
    /// snapshot state nor changes the queue, replay, target, or pruning horizon.
    pub fn worker_status(&self) -> contime_worker::WorkerStatus<I::Time> {
        self.driver.status(0)
    }
    /// Queued messages, including a held prune request; excludes scheduled replay.
    pub fn queued(&self) -> usize {
        self.driver.queued()
    }
    /// Drain asynchronous insertion rejections accumulated during processing.
    /// Multiple rejected routes may report the same event ID. This does not
    /// indicate replay completion. Drain regularly in a running application.
    pub fn take_rejections(&mut self) -> Vec<InsertionRejection> {
        std::mem::take(&mut *self.rejections.borrow_mut())
    }
}
