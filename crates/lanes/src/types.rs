/// One statically known family of borrowed lane events and its batch
/// representation.
pub trait Lanes {
    /// One projected or transient event, optionally borrowing canonical input
    /// for `'a`. Cloning may copy a borrowed view rather than its payload.
    type Event<'a>: Clone
    where
        Self: 'a;

    /// A clonable batch whose iteration preserves the lane's event order.
    /// Lazy iterators and owned collections can both serve as batch types.
    type Batch<'a>: Clone + IntoIterator<Item = Self::Event<'a>>
    where
        Self: 'a;
}

/// Projects canonical raw events into one snapshot's filter lanes.
pub trait FilterLanes<R>: Lanes {
    /// Projects the caller's complete timestamp batch into this snapshot's
    /// filter input, preserving the canonical order of selected events.
    ///
    /// The result may borrow the supplied event references for `'a`. Projection
    /// may omit unrelated variants; metadata still counts raw history events.
    fn project<'a>(events: &'a [&'a R]) -> Self::Batch<'a>;
}

/// Marks a lane family as valid transient input to snapshot application.
pub trait ApplyLanes: Lanes {}

/// One complete canonical timestamp batch before snapshot-specific
/// projection.
#[derive(Clone, Debug)]
pub struct RawBatch<'a, T, R> {
    /// Destination snapshot identity, preserved by projection and application.
    pub snapshot_id: u128,
    /// Timestamp shared by all raw events in this complete batch.
    pub time: T,
    /// Cumulative canonical history count through this batch, before projection
    /// or filtering. This is not the number of events in `events`.
    pub history_event_count: u64,
    /// Canonically ordered raw events at `time`, borrowed for this operation.
    pub events: &'a [&'a R],
}

/// One complete snapshot-specific filter-lane batch.
#[derive(Clone, Debug)]
pub struct FilterBatch<T, B> {
    /// Destination snapshot identity inherited from the raw batch.
    pub snapshot_id: u128,
    /// Complete timestamp boundary represented by this batch.
    pub time: T,
    /// Cumulative raw history count through this batch, unchanged by projection.
    /// It includes raw events omitted from the filter lanes.
    pub history_event_count: u64,
    /// Snapshot-specific projected events in canonical input order.
    pub events: B,
}

/// One complete transient batch delivered to snapshot application.
#[derive(Clone, Debug)]
pub struct ApplyBatch<T, B> {
    /// Destination snapshot identity inherited from the filter input.
    pub snapshot_id: u128,
    /// Complete timestamp boundary, even when filtering produces no events.
    pub time: T,
    /// Cumulative raw history count through this batch. Filtering and decoration
    /// do not change it, so it need not equal the apply-event count.
    pub history_event_count: u64,
    /// Transient filter output, consumed synchronously by snapshot application.
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
    /// Delivers the single effective batch, which may be empty or borrow input.
    ///
    /// A filter must call this exactly once per invocation. The receiver used by
    /// [`crate::contime_lanes::apply()`] applies it immediately and panics on a second call.
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
    /// Transforms one complete input timestamp and calls [`FilterOutput::output`]
    /// exactly once, including when all events are filtered out.
    ///
    /// Output can select, decorate, or combine events for application. Snapshot
    /// identity, time, and raw history count remain those of the input batch.
    /// Replay requires deterministic results for equivalent input and state.
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
    /// Consumes the effective events and updates snapshot state for this complete
    /// timestamp. Empty output still carries time and raw-history progress.
    ///
    /// Borrowed events are scoped to the call and must not be retained beyond
    /// their lifetime. Historical replay requires deterministic application.
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
    /// Returns the destination identity used to group events into one history.
    /// It must agree with the identity installed by [`Self::initialize_snapshot`].
    fn snapshot_id(&self) -> u128;

    /// Initializes identity fields only. Implementations must not advance the
    /// default snapshot timestamp.
    fn initialize_snapshot(&self, snapshot: &mut S);

    /// Constructs `S::default()` and initializes its identity from this route.
    /// The default implementation leaves time at the snapshot's default value.
    fn create_snapshot(&self) -> S
    where
        Self: Sized,
    {
        crate::contime_lanes::route::create_snapshot(self)
    }
}
