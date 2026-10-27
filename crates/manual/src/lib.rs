//! Drive one existing ConTime worker synchronously from the host loop.
//!
//! Enqueue caller-owned worker messages, then call [`ManualConTime::process`] once per host-loop turn.
//! [`ManualConTime::process_with_clock`] is an optional budgeted helper.
//! Queries, replay calls, and per-history pruning calls finish normally. Limits
//! are checked between calls; no snapshot operation is suspended. The worker owns
//! all histories and scheduling. Use its public protocol and storage contracts.
//!
//! Pruning is a barrier: prior replay finishes before authorization, and later
//! messages wait for the prune pass. A horizon beyond the accepted processing
//! target returns [`ProcessError`]. User callbacks and response handlers must not
//! block waiting for this same driver. Panics propagate; discard a driver after
//! catching a storage/application panic, just as for the native worker.
#![deny(missing_docs)]
use contime_worker::{
    AdvanceInput, ApplyInput, Completion, EventQueryInput, EventQueryResponse, MessageWorker, RouteInput, SnapshotListenInput,
    SnapshotQueryInput, SnapshotQueryResponse, SnapshotStore, WorkInput, WorkInputKind,
};
use std::{collections::VecDeque, num::NonZeroUsize, time::Duration};

/// Limits between complete worker operations. Either zero limit pauses work.
#[derive(Clone, Copy, Debug)]
pub struct FrameBudget {
    /// Soft elapsed-time limit; a single operation may overrun it.
    pub duration: Duration,
    /// Maximum message, replay, or per-history pruning calls.
    pub max_steps: usize,
}
/// Why a processing call returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    /// A work-count or elapsed-time limit was reached; more work may remain.
    BudgetExhausted,
    /// No messages, pruning, or replay are currently runnable at the target.
    NoReadyWork,
}
/// Measurements from a processing call, excluding host submission and rendering.
#[derive(Clone, Copy, Debug)]
pub struct FrameReport {
    /// Single-step progress operations performed.
    pub steps: usize,
    /// Real elapsed time supplied by the host clock.
    pub elapsed: Duration,
    /// Whether elapsed time exceeded the soft limit.
    pub overrun: bool,
    /// Messages still waiting, including a held prune request. Not a replay count.
    pub queued: usize,
    /// Distinguishes an exhausted budget from no currently runnable work.
    pub stopped: StopReason,
}
/// A prune horizon exceeded the processing target when its barrier was reached.
/// The rejected message is consumed, its completion released, and history unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessError;
impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("prune horizon exceeds the processing target")
    }
}
impl std::error::Error for ProcessError {}

type Input<M> = <<<M as WorkInput>::Apply as ApplyInput>::Route as RouteInput>::Input;
type Receipt<M> = <<M as WorkInput>::Apply as ApplyInput>::Completion;

type Kind<M> = WorkInputKind<
    <M as WorkInput>::Apply,
    <M as WorkInput>::SnapshotQuery,
    <M as WorkInput>::EventQuery,
    <M as WorkInput>::SnapshotListen,
    <M as WorkInput>::Advance,
>;
struct Delivered<M: WorkInput>(Kind<M>);
impl<M: WorkInput> WorkInput for Delivered<M> {
    type Apply = M::Apply;
    type SnapshotQuery = M::SnapshotQuery;
    type EventQuery = M::EventQuery;
    type SnapshotListen = M::SnapshotListen;
    type Advance = M::Advance;
    fn into_kind(self) -> WorkInputKind<Self::Apply, Self::SnapshotQuery, Self::EventQuery, Self::SnapshotListen, Self::Advance> {
        self.0
    }
}

/// A bounded message queue and one existing, channel-free worker.
///
/// `M` and `S` implement the worker's existing message and storage contracts.
/// Control payloads must be cloneable so targets can be inspected without
/// consuming their original completion ownership. No Send/Sync bounds are added.
/// Capacity bounds queued messages, not retained history or external results.
pub struct ManualConTime<M, S>
where
    M: WorkInput + 'static,
    M::Apply: ApplyInput,
    S: SnapshotStore<Input<M>>,
    M::SnapshotListen: SnapshotListenInput<Time = S::Time>,
    M::Advance: AdvanceInput<Time = S::Time>,
{
    worker: MessageWorker<'static, Delivered<M>, S>,
    queue: VecDeque<M>,
    capacity: usize,
    target: S::Time,
    held_prune: Option<M::Advance>,
    pruning: bool,
    receive_left: Option<usize>,
}
impl<M, S> ManualConTime<M, S>
where
    M: WorkInput + 'static,
    M::Apply: ApplyInput,
    S: SnapshotStore<Input<M>>,
    Receipt<M>: Completion<S::Rejection>,
    Input<M>: Clone,
    M::SnapshotQuery: SnapshotQueryInput<Time = S::Time>,
    <M::SnapshotQuery as SnapshotQueryInput>::Response: SnapshotQueryResponse<S::Snapshot>,
    M::EventQuery: EventQueryInput<Time = S::Time>,
    <M::EventQuery as EventQueryInput>::Response: EventQueryResponse<Input<M>>,
    M::SnapshotListen: SnapshotListenInput<Time = S::Time>,
    M::Advance: AdvanceInput<Time = S::Time> + Clone,
{
    /// Construct one worker without starting threads or binding receive channels.
    pub fn new(capacity: NonZeroUsize, config: S::Config, context: S::Context) -> Self {
        Self {
            worker: MessageWorker::new_inline(config, context),
            queue: VecDeque::new(),
            capacity: capacity.get(),
            target: S::Time::default(),
            held_prune: None,
            pruning: false,
            receive_left: Some(0),
        }
    }
    /// Queue a worker message. A full queue returns the unchanged message.
    pub fn enqueue(&mut self, message: M) -> Result<(), M> {
        if self.queued() >= self.capacity {
            return Err(message);
        }
        self.queue.push_back(message);
        Ok(())
    }
    /// Queued messages, including a held prune request; excludes scheduled replay.
    pub fn queued(&self) -> usize {
        self.queue.len() + usize::from(self.held_prune.is_some())
    }
    /// Repeatedly call [`Self::process`] within a maximum time/work allowance.
    /// Returns immediately when no work is ready; never sleeps or waits for input.
    /// The time limit is checked before each step. A complete step may overrun it;
    /// it is a soft maximum, not a duration to fill or a hard interruption deadline.
    /// A bounded ready-message batch is followed by a replay/pruning opportunity,
    /// even across calls with a one-step budget. Response handling is synchronous.
    pub fn process_with_clock(&mut self, budget: FrameBudget, mut clock: impl FnMut() -> Duration) -> Result<FrameReport, ProcessError> {
        let start = clock();
        let mut steps = 0;
        let mut stopped = StopReason::BudgetExhausted;
        while steps < budget.max_steps && clock().saturating_sub(start) < budget.duration {
            if !self.process()? {
                stopped = StopReason::NoReadyWork;
                break;
            }
            steps += 1;
        }
        let elapsed = clock().saturating_sub(start);
        Ok(FrameReport { steps, elapsed, overrun: elapsed > budget.duration, queued: self.queued(), stopped })
    }
    /// Make one unit of progress and return control to the host.
    ///
    /// Returns `true` after handling one queued message, performing one complete
    /// replay/pruning call, or advancing a pruning barrier. Returns `false` when
    /// nothing is currently runnable; future events may remain stored. One step
    /// can process multiple events. This method never sleeps or waits for input.
    pub fn process(&mut self) -> Result<bool, ProcessError> {
        loop {
            if self.held_prune.is_some() {
                if self.worker.prune_step() || self.worker.step() {
                    return Ok(true);
                }
                let prune = self.held_prune.take().unwrap();
                self.worker.handle(Delivered(WorkInputKind::Prune(prune)));
                self.pruning = true;
                return Ok(true);
            }
            if self.pruning {
                if self.worker.prune_step() {
                    return Ok(true);
                }
                self.pruning = false;
            }
            if let Some(left) = self.receive_left {
                let left = if left == 0 { self.queue.len().max(1) } else { left };
                if let Some(message) = self.queue.pop_front() {
                    self.receive_left = if left > 1 { Some(left - 1) } else { None };
                    let kind = message.into_kind();
                    match kind {
                        WorkInputKind::Advance(ref advance) => {
                            let (time, completion) = advance.clone().into_parts();
                            drop(completion);
                            self.target = self.target.clone().max(time);
                        }
                        WorkInputKind::Prune(prune) => {
                            let (horizon, completion) = prune.clone().into_parts();
                            drop(completion);
                            if horizon > self.target {
                                return Err(ProcessError);
                            }
                            self.held_prune = Some(prune);
                            return Ok(true);
                        }
                        _ => {}
                    }
                    self.worker.handle(Delivered(kind));
                    return Ok(true);
                }
                self.receive_left = None;
            }
            let worked = self.worker.prune_step() || self.worker.step();
            self.receive_left = Some(0);
            if worked {
                return Ok(true);
            }
            if self.queue.is_empty() {
                return Ok(false);
            }
        }
    }
}
