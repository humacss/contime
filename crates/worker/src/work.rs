use std::collections::{BTreeSet, VecDeque};
use std::ops::Bound::{Excluded, Unbounded};

use ahash::{AHashMap, AHashSet};
use crossbeam_channel::{Receiver, TryRecvError};

use crate::listen::NotificationCollections;
use crate::query::{query_events, query_snapshots};
use crate::types::{
    AdvanceInput, ApplyInput, Completion, Coordination, EventQueryInput, EventQueryResponse, ReplayUpdate, RouteInput, SnapshotListenInput,
    SnapshotQueryInput, SnapshotQueryResponse, SnapshotStore, StoreSlot, WorkInput, WorkInputKind,
};

type ApplyRoute<M> = <<M as WorkInput>::Apply as ApplyInput>::Route;
type ApplyCompletion<M> = <<M as WorkInput>::Apply as ApplyInput>::Completion;
type ApplyEvent<M> = <ApplyRoute<M> as RouteInput>::Input;
type EventTime<M, S> = <S as SnapshotStore<ApplyEvent<M>>>::Time;
type PendingPrune<T, C> = (T, std::vec::IntoIter<u128>, C);
/// Drives messages, incremental pruning, and replay on the calling thread.
///
/// Drains each bounded batch of already-ready messages before granting replay
/// opportunities, checking newly queued messages between steps. Pruning has
/// priority over replay. Queries do not advance the replay schedule or publish
/// effects; sustained queries still allow processing opportunities.
///
/// `registrations` accepts activity subscribers (`false` idle, `true` working)
/// only in the idle phase. Idle follows completed hooks, notifications, and
/// queued pruning. Disconnected subscribers are removed; bounded subscriber
/// channels can block notification. Pass [`crossbeam_channel::never`] when no
/// subscriptions are needed.
///
/// Drained work-channel disconnection returns after available replay/pruning
/// steps finish. Retained events beyond the processing target do not prevent
/// return or idle. A coordinated round awaiting resolution cannot replay until
/// its matching resolution arrives. Store or callback panics propagate.
/// Only explicit prune or matching resolve messages authorize history removal.
pub fn work_messages<M, S>(
    input: Receiver<M>,
    store_config: S::Config,
    store_context: S::Context,
    registrations: Receiver<crossbeam_channel::Sender<bool>>,
    coordination: Option<Coordination<EventTime<M, S>>>,
) where
    M: WorkInput,
    M::Apply: ApplyInput,
    S: SnapshotStore<ApplyEvent<M>>,
    ApplyCompletion<M>: Completion<S::Rejection>,
    M::SnapshotQuery: SnapshotQueryInput<Time = S::Time>,
    <M::SnapshotQuery as SnapshotQueryInput>::Response: SnapshotQueryResponse<S::Snapshot>,
    M::EventQuery: EventQueryInput<Time = S::Time>,
    <M::EventQuery as EventQueryInput>::Response: EventQueryResponse<ApplyEvent<M>>,
    M::SnapshotListen: SnapshotListenInput<Time = S::Time>,
    M::Advance: AdvanceInput<Time = S::Time>,
    ApplyEvent<M>: Clone,
{
    let mut worker = MessageWorker::<M, S>::new(&input, &registrations, store_config, store_context, coordination);
    worker.activity_loop();
}

/// Worker state that can be driven synchronously or by [`work_messages`].
///
/// A synchronous driver delivers messages with [`Self::handle`], then gives
/// [`Self::prune_step`] priority over [`Self::step`], checking for delivered
/// messages between steps. No thread is started by this type.
pub struct MessageWorker<'a, M, S>
where
    M: WorkInput,
    M::Apply: ApplyInput,
    S: SnapshotStore<ApplyEvent<M>>,
    M::SnapshotListen: SnapshotListenInput<Time = S::Time>,
    M::Advance: AdvanceInput<Time = S::Time>,
{
    input: &'a Receiver<M>,
    registrations: &'a Receiver<crossbeam_channel::Sender<bool>>,
    snapshots: AHashMap<u128, StoreSlot<S>>,
    listeners: NotificationCollections<S::Time, <M::SnapshotListen as SnapshotListenInput>::Listener>,
    activity_listeners: Vec<crossbeam_channel::Sender<bool>>,
    store_config: S::Config,
    store_context: S::Context,
    current_time: S::Time,
    horizon: S::Time,
    pending_prunes: VecDeque<PendingPrune<S::Time, <M::Advance as AdvanceInput>::Completion>>,
    schedule: BTreeSet<(S::Time, u128)>,
    pending: AHashMap<u128, S::Time>,
    coordination: Option<Coordination<S::Time>>,
    fence_round: Option<u64>,
    fence_routers: AHashSet<usize>,
    reported_round: Option<u64>,
    awaiting_resolution: Option<u64>,
    process_before_report: bool,
}

impl<'a, M, S> MessageWorker<'a, M, S>
where
    M: WorkInput,
    M::Apply: ApplyInput,
    S: SnapshotStore<ApplyEvent<M>>,
    ApplyCompletion<M>: Completion<S::Rejection>,
    M::SnapshotQuery: SnapshotQueryInput<Time = S::Time>,
    <M::SnapshotQuery as SnapshotQueryInput>::Response: SnapshotQueryResponse<S::Snapshot>,
    M::EventQuery: EventQueryInput<Time = S::Time>,
    <M::EventQuery as EventQueryInput>::Response: EventQueryResponse<ApplyEvent<M>>,
    M::SnapshotListen: SnapshotListenInput<Time = S::Time>,
    M::Advance: AdvanceInput<Time = S::Time>,
    ApplyEvent<M>: Clone,
{
    /// Constructs empty worker state, taking ownership of store configuration
    /// and context while borrowing the input and registration receivers.
    ///
    /// Processing target and admission horizon start at the time type’s default.
    /// Stores are created lazily on first insertion; listener registration alone
    /// creates no store. Construction does not read channels or publish activity.
    pub fn new(
        input: &'a Receiver<M>,
        registrations: &'a Receiver<crossbeam_channel::Sender<bool>>,
        store_config: S::Config,
        store_context: S::Context,
        coordination: Option<Coordination<S::Time>>,
    ) -> Self {
        Self {
            input,
            registrations,
            snapshots: AHashMap::new(),
            listeners: NotificationCollections::new(),
            activity_listeners: Vec::new(),
            current_time: S::Time::default(),
            horizon: S::Time::default(),
            pending_prunes: VecDeque::new(),
            schedule: BTreeSet::new(),
            pending: AHashMap::new(),
            coordination,
            fence_round: None,
            fence_routers: AHashSet::new(),
            reported_round: None,
            awaiting_resolution: None,
            process_before_report: false,
            store_config,
            store_context,
        }
    }

    /// Waits and reports activity without consuming work.
    fn activity_loop(&mut self) {
        let mut ready = crossbeam_channel::Select::new();
        let work_ready = ready.recv(self.input);
        let subscription = ready.recv(self.registrations);
        loop {
            if ready.ready() != work_ready {
                match self.registrations.try_recv() {
                    Ok(listener) => {
                        if listener.send(false).is_ok() {
                            self.activity_listeners.push(listener);
                        }
                    }
                    Err(TryRecvError::Disconnected) => ready.remove(subscription),
                    Err(TryRecvError::Empty) => {}
                }
                continue;
            }
            self.activity_listeners.retain(|listener| listener.send(true).is_ok());
            if self.work_loop() == TryRecvError::Disconnected {
                return;
            }
            self.activity_listeners.retain(|listener| listener.send(false).is_ok());
        }
    }

    /// Drains each ready batch before giving scheduled work a processing turn.
    fn work_loop(&mut self) -> TryRecvError {
        let mut batch_size = self.input.len().max(1);
        let mut remaining = batch_size;
        loop {
            let message = match self.input.try_recv() {
                Ok(message) => message,
                Err(reason) => {
                    if self.prune_step() || self.step() {
                        continue;
                    }
                    return reason;
                }
            };
            self.handle(message);
            remaining -= 1;
            if remaining == 0 {
                // Queries and other producers may replenish the queue forever.
                // Preserve batching for already-ready messages without requiring
                // the queue to become empty before replay or pruning can run.
                for _ in 0..batch_size {
                    if self.prune_step() || !self.step() {
                        break;
                    }
                    // Replay callbacks may enqueue queries or late inputs.
                    // Service them before starting another replay step.
                    if !self.input.is_empty() {
                        break;
                    }
                }
                batch_size = self.input.len().max(1);
                remaining = batch_size;
            }
        }
    }

    /// Consumes one delivered message without running queued replay or pruning.
    ///
    /// Apply completion covers insertion and rejection collection, and advance
    /// completion covers target acceptance. Queries and listener installation
    /// happen immediately; prune completion remains owned until the pass ends.
    /// Stale resolutions are dropped without changing the awaited round.
    ///
    /// # Panics
    ///
    /// Panics if a newly authorized horizon would discard pending replay.
    /// Storage, listener, and coordination callback panics also propagate.
    pub fn handle(&mut self, message: M) {
        match message.into_kind() {
            WorkInputKind::Apply(batch) => {
                self.apply_batch(batch);
            }
            WorkInputKind::SnapshotQuery(query) => {
                query_snapshots::<_, ApplyEvent<M>, _>(query, &mut self.snapshots, &mut self.store_context)
            }
            WorkInputKind::EventQuery(query) => query_events::<_, ApplyEvent<M>, _>(query, &self.snapshots),
            WorkInputKind::SnapshotListen(registration) => {
                let (time, snapshot_ids, listener) = registration.into_parts();
                self.listeners.register(time, snapshot_ids, listener, &mut self.snapshots);
            }
            WorkInputKind::Advance(advance) => {
                let (target_time, completion) = advance.into_parts();
                self.current_time = self.current_time.clone().max(target_time);
                drop(completion);
            }
            WorkInputKind::Fence { round, router } => self.fence(round, router),
            WorkInputKind::Resolve { round, prune } => {
                if self.awaiting_resolution == Some(round) {
                    self.awaiting_resolution = None;
                    self.queue_prune(prune);
                    self.process_before_report = !self.schedule.is_empty();
                }
            }
            WorkInputKind::Prune(prune) => {
                self.queue_prune(prune);
            }
        }
    }

    fn queue_prune(&mut self, prune: M::Advance) {
        let (horizon, completion) = prune.into_parts();
        if horizon > self.horizon {
            assert!(self.schedule.iter().all(|(time, _)| time >= &horizon), "prune would discard pending replay");
            let snapshots = self.snapshots.keys().copied().collect::<Vec<_>>().into_iter();
            self.horizon = horizon.clone();
            self.pending_prunes.push_back((horizon, snapshots, completion));
        } else if !self.pending_prunes.is_empty() {
            self.pending_prunes.push_back((horizon, Vec::new().into_iter(), completion));
        }
    }

    fn apply_batch(&mut self, batch: M::Apply) {
        let (routes, completion) = batch.into_parts();
        let mut rejections = Vec::new();
        for route in routes {
            let (snapshot_id, input) = route.into_parts();
            let time = S::event_time(&input);
            let slot = self.snapshots.entry(snapshot_id).or_insert_with(StoreSlot::metadata_only);
            let store = slot.store.get_or_insert_with(|| S::create(snapshot_id, &self.store_config, &self.horizon));
            let result = store.insert(input, &self.horizon);
            rejections.extend(result.rejections);
            if result.changed {
                let pending = self.pending.get(&snapshot_id).cloned().map_or(time.clone(), |old| old.min(time));
                self.schedule_at(snapshot_id, pending);
            }
        }
        if !rejections.is_empty() {
            completion.reject(rejections);
        }
    }

    fn schedule_at(&mut self, snapshot_id: u128, time: S::Time) {
        if let Some(previous) = self.pending.insert(snapshot_id, time.clone()) {
            self.schedule.remove(&(previous, snapshot_id));
        }
        self.schedule.insert((time, snapshot_id));
    }

    /// Processes one scheduled snapshot through the next distinct scheduled
    /// timestamp bucket or processing target, whichever comes first, inclusively.
    ///
    /// Returns `true` when a store processing call ran. Returns `false` when no
    /// replay is available at the target, pruning is pending, or a reported
    /// coordination round awaits resolution. Equal-time entries run by snapshot
    /// ID. Remaining replay is scheduled at its earliest application time;
    /// a completed history leaves the schedule. One call may process
    /// multiple event timestamps; replay notifications flush before returning.
    /// A step may also publish a ready coordination report and pause later replay.
    ///
    /// # Panics
    ///
    /// Panics if storage reports remaining replay at or before the endpoint
    /// it just finished processing.
    pub fn step(&mut self) -> bool {
        if self.awaiting_resolution.is_some() || !self.pending_prunes.is_empty() {
            return false;
        }
        let worked = self.process_step();
        // A zero-interval coordinator must not pause us repeatedly without
        // allowing progress. One step (including a future-only no-op) suffices.
        self.process_before_report = false;
        self.report_if_ready();
        worked
    }

    fn process_step(&mut self) -> bool {
        let Some((time, snapshot_id)) = self.schedule.first().cloned() else { return false };
        if time > self.current_time {
            return false;
        }
        let end = self
            .schedule
            .range((Excluded((time.clone(), u128::MAX)), Unbounded))
            .next()
            .map_or_else(|| self.current_time.clone(), |(next, _)| next.clone().min(self.current_time.clone()));
        self.schedule.pop_first();
        self.pending.remove(&snapshot_id);
        let slot = self.snapshots.get_mut(&snapshot_id).expect("scheduled history exists");
        let store = slot.store.as_mut().expect("scheduled store exists");
        store.process_until(&end, &mut self.store_context);
        if let Some(next) = store.earliest_replay_time() {
            assert!(next > end, "storage must finish replay through the inclusive processing endpoint");
            self.schedule_at(snapshot_id, next);
        }
        self.listeners.record(ReplayUpdate { snapshot_id, affected_from: time }, &mut self.snapshots);
        self.listeners.flush();
        true
    }

    fn fence(&mut self, round: u64, router: usize) {
        let Some(coordination) = self.coordination.as_mut() else { return };
        if self.reported_round.is_some_and(|reported| round <= reported)
            || self.fence_round.is_some_and(|current| round < current)
            || router >= coordination.router_count
        {
            return;
        }
        if self.fence_round != Some(round) {
            self.fence_round = Some(round);
            self.fence_routers.clear();
        }
        self.fence_routers.insert(router);
        self.report_if_ready();
    }

    fn report_if_ready(&mut self) {
        let Some(round) = self.fence_round else { return };
        if self.process_before_report || self.reported_round.is_some_and(|reported| round <= reported) {
            return;
        }
        let Some(coordination) = self.coordination.as_mut() else { return };
        if self.fence_routers.len() == coordination.router_count {
            self.reported_round = Some(round);
            self.awaiting_resolution = Some(round);
            let earliest = self.pending.keys().filter_map(|id| self.snapshots[id].store.as_ref()?.earliest_replay_time()).min();
            (coordination.report)(round, earliest);
        }
    }

    /// Forwards and prunes at most one captured history in the oldest prune job.
    ///
    /// Returns `true` when a job was serviced, including an empty pass, and
    /// `false` when none is queued. Completion stays owned until every captured
    /// history and its forwarding hooks finish. The completed-horizon callback
    /// runs before that ownership is released; callback/store panics propagate
    /// without reporting successful completion. Service messages between calls.
    ///
    /// # Panics
    ///
    /// Panics if pruning a history would discard its pending replay.
    pub fn prune_step(&mut self) -> bool {
        let Some((horizon, snapshots, _)) = self.pending_prunes.front_mut() else { return false };
        if let Some(snapshot_id) = snapshots.next() {
            let slot = self.snapshots.get_mut(&snapshot_id).expect("pruning history exists");
            if let Some(store) = slot.store.as_mut() {
                assert!(self.pending.get(&snapshot_id).is_none_or(|time| time >= horizon), "prune would discard pending replay");
                store.forward(horizon, &mut self.store_context);
                store.prune();
            }
        }
        if snapshots.len() == 0 {
            if let Some(coordination) = self.coordination.as_mut() {
                (coordination.pruned)(horizon.clone());
            }
            self.pending_prunes.pop_front();
        }
        true
    }
}
