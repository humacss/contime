use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// Worker-local replay scheduling policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerConfig {
    /// Maximum time changed events may wait before their snapshot is replayed.
    pub maximum_dirty_age: Duration,
    /// Maximum non-overdue snapshots replayed after each received batch.
    pub replays_per_receive: usize,
    /// Deadline entries tolerated before relative compaction is considered.
    pub deadline_compaction_minimum: usize,
    /// Maximum deadline entries per active snapshot before compaction.
    pub deadline_compaction_multiplier: usize,
}

/// One event route already assigned to this worker.
#[derive(Debug)]
pub struct RoutedInput<I> {
    pub snapshot_id: u128,
    pub input: I,
}

/// A caller-selected route consumed by a worker.
pub trait RouteInput {
    type Input;

    fn into_parts(self) -> (u128, Self::Input);
}

impl<I> RouteInput for RoutedInput<I> {
    type Input = I;

    fn into_parts(self) -> (u128, Self::Input) {
        (self.snapshot_id, self.input)
    }
}

/// One worker apply message.
#[derive(Debug)]
pub struct ApplyBatch<I, C> {
    pub inputs: Vec<RoutedInput<I>>,
    pub completion: C,
}

/// A caller-selected apply message consumed by a worker.
pub trait ApplyInput {
    type Route: RouteInput;
    type Completion;

    fn into_parts(self) -> (Vec<Self::Route>, Self::Completion);
}

impl<I, C> ApplyInput for ApplyBatch<I, C> {
    type Route = RoutedInput<I>;
    type Completion = C;

    fn into_parts(self) -> (Vec<Self::Route>, Self::Completion) {
        (self.inputs, self.completion)
    }
}

/// A request-scoped response handle forwarded opaquely through the router.
///
/// Successful completion is represented by dropping the handle without
/// calling `reject`.
pub trait Completion<R> {
    fn reject(self, rejections: Vec<R>);
}

impl<R> Completion<R> for crossbeam_channel::Sender<Vec<R>> {
    fn reject(self, rejections: Vec<R>) {
        let _ = self.send(rejections);
    }
}

/// The result of inserting one input into the canonical event store.
#[derive(Debug)]
pub struct EventInsert<R> {
    pub changed: bool,
    pub rejections: Vec<R>,
}

/// One snapshot whose completed replay may affect every time at or after
/// `affected_from`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayUpdate<T> {
    pub snapshot_id: u128,
    pub affected_from: T,
}

/// Timestamp arithmetic required for worker-local horizon advancement.
pub trait AdvanceTime: Clone + Default + Ord {
    fn saturating_sub(&self, retention: &Self) -> Self;
}

macro_rules! impl_advance_time {
    ($($time:ty),+ $(,)?) => {
        $(
            impl AdvanceTime for $time {
                fn saturating_sub(&self, retention: &Self) -> Self {
                    <$time>::saturating_sub(*self, *retention)
                }
            }
        )+
    };
}

impl_advance_time!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

pub trait AdvanceInput {
    type Time: AdvanceTime;
    type Completion;

    fn into_parts(self) -> (Self::Time, Self::Completion);
}

pub trait AdvanceOutput<T, C>: Sized {
    fn advance(time: T, completion: C) -> Self;
}

/// Canonical per-snapshot event storage owned by a worker.
pub trait Events<I>: Sized {
    type Config;
    type Rejection;
    type Time: AdvanceTime;

    fn create(snapshot_id: u128, config: &Self::Config, horizon: &Self::Time) -> Self;

    fn insert(&mut self, input: I) -> EventInsert<Self::Rejection>;

    fn dirty_time(&self) -> &Self::Time;

    fn prune_before(&mut self, horizon: &Self::Time);
}

/// Read-only event-history access required by worker queries.
pub trait QueryEvents<I> {
    type Time: Clone + Ord;

    fn clone_between(&self, from: &Self::Time, to: &Self::Time) -> Vec<I>
    where
        I: Clone;
}

/// Materialized per-snapshot checkpoint storage.
///
/// Checkpoint updates can inspect canonical events but cannot modify them.
pub trait Checkpoints<S>: Sized {
    type Config;
    type Context;
    type Time: AdvanceTime;

    fn create(snapshot_id: u128, config: &Self::Config) -> Self;

    fn update(&mut self, events: &mut S, context: &mut Self::Context) -> Self::Time;

    fn advance_before(&mut self, events: &S, context: &mut Self::Context, horizon: &Self::Time);
}

/// Read-only checkpoint reconstruction required by worker queries.
pub trait QueryCheckpoints<E> {
    type Context;
    type Time: Clone + Ord;
    type Snapshot;

    fn query_at(&self, events: &E, context: &mut Self::Context, time: Self::Time) -> Option<Box<Self::Snapshot>>;
}

pub trait SnapshotQueryInput {
    type Time;
    type Response;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response);
}

pub trait EventQueryInput {
    type Time;
    type Response;

    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response);
}

/// A worker-owned timestamped snapshot replay listener.
pub trait SnapshotListener<T>: Clone {
    /// Reports that this listener collection was installed.
    /// Returns `false` when the receiver no longer exists.
    fn registered(&self, time: T, snapshot_ids: Vec<u128>) -> bool;

    /// Reports the collection members completed in one worker replay batch.
    /// Returns `false` when the receiver no longer exists.
    fn replayed(&self, time: T, snapshot_ids: Vec<u128>) -> bool;
}

/// Caller-selected snapshot-listener registration consumed by a worker.
pub trait SnapshotListenInput {
    type Time: Clone + Ord;
    type Listener: SnapshotListener<Self::Time>;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener);
}

pub trait SnapshotQueryResponse<S> {
    fn send(self, snapshots: Vec<Box<S>>);
}

impl<S> SnapshotQueryResponse<S> for crossbeam_channel::Sender<Vec<Box<S>>> {
    fn send(self, snapshots: Vec<Box<S>>) {
        let _ = crossbeam_channel::Sender::send(&self, snapshots);
    }
}

pub trait EventQueryResponse<I> {
    fn send(self, events: Vec<I>);
}

impl<I> EventQueryResponse<I> for crossbeam_channel::Sender<Vec<I>> {
    fn send(self, events: Vec<I>) {
        let _ = crossbeam_channel::Sender::send(&self, events);
    }
}

pub enum WorkInputKind<A, SQ, EQ, SL, AD> {
    Apply(A),
    SnapshotQuery(SQ),
    EventQuery(EQ),
    SnapshotListen(SL),
    Advance(AD),
}

pub trait WorkInput {
    type Apply;
    type SnapshotQuery;
    type EventQuery;
    type SnapshotListen;
    type Advance;

    fn into_kind(self) -> WorkInputKind<Self::Apply, Self::SnapshotQuery, Self::EventQuery, Self::SnapshotListen, Self::Advance>;
}

pub(crate) type Request<C, R> = Rc<RefCell<PendingRequest<C, R>>>;

/// One generational index into worker-local notification collection storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NotificationId {
    pub(crate) index: usize,
    pub(crate) generation: u64,
}

pub(crate) struct SnapshotSlot<S, K, C, R> {
    pub(crate) events: Option<S>,
    pub(crate) checkpoints: Option<K>,
    pub(crate) waiters: Vec<Request<C, R>>,
    pub(crate) notification_ids: Vec<NotificationId>,
}

impl<S, K, C, R> SnapshotSlot<S, K, C, R> {
    pub(crate) const fn metadata_only() -> Self {
        Self { events: None, checkpoints: None, waiters: Vec::new(), notification_ids: Vec::new() }
    }

    #[cfg(test)]
    pub(crate) fn with_events(events: S) -> Self {
        Self { events: Some(events), checkpoints: None, waiters: Vec::new(), notification_ids: Vec::new() }
    }
}

pub(crate) struct PendingRequest<C, R> {
    pub(crate) completion: Option<C>,
    pub(crate) pending_snapshots: usize,
    pub(crate) rejections: Vec<R>,
}

pub(crate) fn new_request<C, R>(completion: C) -> Request<C, R> {
    Rc::new(RefCell::new(PendingRequest { completion: Some(completion), pending_snapshots: 0, rejections: Vec::new() }))
}

pub(crate) fn register_waiter<S, K, C, R>(slot: &mut SnapshotSlot<S, K, C, R>, request: &Request<C, R>) {
    if slot.waiters.iter().any(|waiter| Rc::ptr_eq(waiter, request)) {
        return;
    }

    request.borrow_mut().pending_snapshots += 1;
    slot.waiters.push(Rc::clone(request));
}

pub(crate) fn complete_snapshot<C, R>(request: Request<C, R>)
where
    C: Completion<R>,
{
    request.borrow_mut().pending_snapshots -= 1;
    finish_if_ready(&request);
}

pub(crate) fn finish_if_ready<C, R>(request: &Request<C, R>)
where
    C: Completion<R>,
{
    let completed = {
        let mut request = request.borrow_mut();
        if request.pending_snapshots != 0 {
            None
        } else {
            request.completion.take().map(|completion| {
                let rejections = std::mem::take(&mut request.rejections);
                (completion, rejections)
            })
        }
    };

    if let Some((completion, rejections)) = completed {
        if rejections.is_empty() {
            drop(completion);
        } else {
            completion.reject(rejections);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SnapshotSlot;

    #[test]
    fn metadata_only_snapshot_slots_do_not_initialize_history_or_checkpoints() {
        let slot = SnapshotSlot::<Vec<u8>, Vec<u8>, (), ()>::metadata_only();

        assert!(slot.events.is_none());
        assert!(slot.checkpoints.is_none());
        assert!(slot.waiters.is_empty());
        assert!(slot.notification_ids.is_empty());
    }
}
