/// One statically known family of borrowed lane events and its batch
/// representation.
pub trait Lanes {
    type Event<'a>: Clone
    where
        Self: 'a;

    type Batch<'a>: Clone + IntoIterator<Item = Self::Event<'a>>
    where
        Self: 'a;
}

/// Projects canonical raw events into one snapshot's filter lanes.
pub trait FilterLanes<R>: Lanes {
    fn project<'a>(events: &'a [&'a R]) -> Self::Batch<'a>;
}

/// Marks a lane family as valid transient input to snapshot application.
pub trait ApplyLanes: Lanes {}

/// One complete canonical timestamp batch before snapshot-specific
/// projection.
#[derive(Clone, Debug)]
pub struct RawBatch<'a, T, R> {
    pub snapshot_id: u128,
    pub time: T,
    pub history_event_count: u64,
    pub events: &'a [&'a R],
}

/// One complete snapshot-specific filter-lane batch.
#[derive(Clone, Debug)]
pub struct FilterBatch<T, B> {
    pub snapshot_id: u128,
    pub time: T,
    pub history_event_count: u64,
    pub events: B,
}

/// One complete transient batch delivered to snapshot application.
#[derive(Clone, Debug)]
pub struct ApplyBatch<T, B> {
    pub snapshot_id: u128,
    pub time: T,
    pub history_event_count: u64,
    pub events: B,
}

/// Scoped receiver for one filter's transient apply-lane output.
///
/// Snapshot ID, timestamp, and raw history count stay under the caller's
/// control; a filter selects only the effective event batch.
pub trait FilterOutput<A>
where
    A: ApplyLanes,
{
    fn output<'a>(&mut self, events: A::Batch<'a>)
    where
        A: 'a;
}

/// Deterministically transforms a snapshot-specific filter batch into its
/// apply-lane batch.
pub trait EventFilter<T, F, A>
where
    F: Lanes,
    A: ApplyLanes,
{
    fn filter<'a, O>(&self, batch: FilterBatch<T, F::Batch<'a>>, output: &mut O)
    where
        F: 'a,
        A: 'a,
        O: FilterOutput<A>;
}

/// Applies one filter-produced transient lane batch to snapshot state.
pub trait ApplyEvents<T, A>
where
    A: ApplyLanes,
{
    fn apply_events<'a>(&mut self, batch: ApplyBatch<T, A::Batch<'a>>)
    where
        A: 'a;
}

pub(crate) struct SnapshotOutput<'a, S, T> {
    pub(crate) snapshot: &'a mut S,
    pub(crate) snapshot_id: u128,
    pub(crate) time: Option<T>,
    pub(crate) history_event_count: u64,
    pub(crate) output_count: usize,
}

/// Allocation-free default filter used when filter and apply lanes coincide.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PassThrough;

impl<T, L> EventFilter<T, L, L> for PassThrough
where
    L: ApplyLanes,
{
    fn filter<'a, O>(&self, batch: FilterBatch<T, L::Batch<'a>>, output: &mut O)
    where
        L: 'a,
        O: FilterOutput<L>,
    {
        output.output(batch.events);
    }
}

/// Routes one canonical raw event to a snapshot and initializes that
/// snapshot's identity on otherwise default state.
pub trait Route<S>
where
    S: Default,
{
    fn snapshot_id(&self) -> u128;

    /// Initializes identity fields only. Implementations must not advance the
    /// default snapshot timestamp.
    fn initialize_snapshot(&self, snapshot: &mut S);

    fn create_snapshot(&self) -> S
    where
        Self: Sized,
    {
        crate::route::create_snapshot(self)
    }
}
