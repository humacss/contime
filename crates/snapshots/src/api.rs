//! Worker-facing access to one snapshot's checkpoint history.
use crate::contime_checkpoints::{
    Apply, Checkpoint, Event, EventStore, ForwardError, Insert, InsertEventStore, NoCheckpoint, Snapshot, Store, Timestamp,
};

/// Owns one snapshot's canonical event history and retained checkpoints.
///
/// Insertion invalidates affected checkpoint state; queries reconstruct clones,
/// processing retains progress, and forwarding establishes the lower retained
/// boundary. Event identity and storage are supplied by `H`.
pub struct SnapshotStore<S: Snapshot, H> {
    store: Store<S, H>,
}

impl<S: Snapshot, H> SnapshotStore<S, H> {
    /// Takes ownership of an event store and its pre-event initial snapshot.
    ///
    /// `snapshot.time()` becomes the initial retained-history horizon. The initial
    /// checkpoint has zero applied events and precedes every supplied event,
    /// including events at that timestamp; construction performs no application.
    ///
    /// `checkpoint_interval` counts raw canonical events, including events ignored
    /// by application logic. Nonzero intervals extend to finish an entire timestamp
    /// bucket; zero leaves each playback interval unbounded. Later calls may retain
    /// a partial interval and extend it when processing resumes.
    pub fn new(events: H, snapshot: S, checkpoint_interval: u64) -> Self {
        Self { store: Store::new(events, snapshot, checkpoint_interval) }
    }

    /// Read-only access for consumer-defined event queries. Admission and
    /// mutation must still go through this store so validity stays synchronized.
    pub fn events(&self) -> &H {
        &self.store.events
    }
}

impl<S: Snapshot, H: EventStore<Time = S::Time>> SnapshotStore<S, H> {
    /// Earliest possible application when resuming dirty history. Includes any
    /// unchanged prefix reconstructed from the selected checkpoint. Does not clone
    /// or apply the snapshot; `None` means no remaining event application in the
    /// currently stored history. Future accepted events can create new work.
    /// This boundary may be earlier than the most recently inserted event.
    pub fn earliest_replay_time(&self) -> Option<S::Time> {
        let start = self.store.dirty.clone().max(self.store.horizon.clone());
        // A completed horizon bucket is valid when measuring remaining work.
        // Insertion at that bucket already invalidates it through Store::insert.
        let index = self.store.starting_index(&start, true).expect("store retains a valid starting checkpoint");
        let checkpoint = &self.store.checkpoints[index];
        let boundary = (checkpoint.history_event_count != 0).then(|| checkpoint.snapshot.time());
        self.store.events.iter_after(boundary).next().map(Event::time)
    }

    /// Admits an event without processing it. Changed history invalidates
    /// retained state at the event's timestamp and afterward.
    ///
    /// Events strictly before the current horizon return [`Insert::BeforeHorizon`]
    /// before the event-store adapter is called. Events exactly at the horizon
    /// remain admissible. Duplicate or rejected events leave checkpoint validity
    /// unchanged; deduplication identity is defined by `H`.
    pub fn insert(&mut self, event: H::Event) -> Insert
    where
        H: InsertEventStore,
    {
        self.store.insert(event)
    }

    /// Physically removes history before the current horizon, except for its
    /// predecessor checkpoint. Can be called independently of forwarding frequency.
    ///
    /// Events exactly at the horizon remain available. This method does not apply
    /// events, invoke forwarding hooks, or move progress or the horizon. Repeating
    /// it without a new horizon performs no additional logical cleanup.
    pub fn prune(&mut self) {
        self.store.prune();
    }

    /// Reconstructs state through the inclusive target without retaining changes.
    /// Supply an effect-free context for query application.
    ///
    /// Returns an owned clone without changing retained checkpoints, dirty progress,
    /// or the horizon. In a gap, the returned snapshot keeps the last applied
    /// timestamp (or its selected starting checkpoint's time), rather than being
    /// stamped with the requested target.
    ///
    /// # Errors
    ///
    /// Returns [`NoCheckpoint`] when the target predates retained history or no
    /// valid starting checkpoint exists.
    pub fn query<C>(&mut self, time: S::Time, context: &C) -> Result<Box<S>, NoCheckpoint>
    where
        H::Event: Apply<Checkpoint<S>, C>,
    {
        crate::contime_checkpoints::query::query_at(&mut self.store, context, time)
    }

    /// Ensures retained state incorporates accepted events through the inclusive target.
    /// Resumes from valid state; replay and checkpoint placement are internal details.
    ///
    /// Every application callback receives a complete timestamp batch. Previously
    /// processed events may be reconstructed after late insertion, so callbacks
    /// must tolerate replay. A target in an event-free gap does not synthesize a
    /// new event timestamp or move the retention horizon.
    ///
    /// # Errors
    ///
    /// Returns [`NoCheckpoint`] when the target predates retained history or no
    /// valid starting checkpoint exists.
    pub fn process_until<C>(&mut self, time: S::Time, context: &C) -> Result<(), NoCheckpoint>
    where
        H::Event: Apply<Checkpoint<S>, C>,
    {
        crate::contime_checkpoints::replay::replay(&mut self.store, context, time)
    }

    /// Processes through the inclusive target and returns raw canonical events
    /// traversed by this replay operation, including events drained after a partial
    /// application callback and repeated prefixes rebuilt after late insertion.
    /// Queries and retention forwarding do not contribute to this count.
    ///
    /// Uses the same replay and checkpoint placement as [`Self::process_until`].
    ///
    /// # Errors
    ///
    /// Returns [`NoCheckpoint`] under the same conditions as [`Self::process_until`].
    #[cfg(feature = "monitor")]
    pub fn process_until_counted<C>(&mut self, time: S::Time, context: &C) -> Result<u64, NoCheckpoint>
    where
        H::Event: Apply<Checkpoint<S>, C>,
    {
        crate::contime_checkpoints::replay::replay_counted(&mut self.store, context, time)
    }

    /// Forwards through the horizon's predecessor, then publishes the horizon.
    /// The hook may compact the snapshot while preserving observable state.
    /// Events and obsolete checkpoints are not physically removed.
    ///
    /// Application includes events through `horizon.previous()`, excluding events
    /// exactly at `horizon`. The hook runs after each newly applied timestamp and
    /// at the final predecessor if needed, even across an event-free gap. It must
    /// preserve observable state and must not advance snapshot time beyond the
    /// supplied hook timestamp. The final retained checkpoint is stamped with the
    /// predecessor before the new horizon is published. Call [`Self::prune`] for
    /// physical cleanup.
    ///
    /// # Errors
    ///
    /// Returns [`ForwardError::NoPreviousTimestamp`] if the horizon has no
    /// predecessor, or [`ForwardError::NoCheckpoint`] if that predecessor cannot
    /// be reconstructed. This includes repeating the current horizon, because its
    /// predecessor is already before the retained boundary.
    ///
    /// Application or hook panics propagate without rolling back effects.
    pub fn forward<C>(&mut self, horizon: S::Time, context: &C, hook: impl FnMut(&mut S, &S::Time, &C)) -> Result<(), ForwardError>
    where
        S::Time: Timestamp,
        H::Event: Apply<Checkpoint<S>, C>,
    {
        crate::contime_checkpoints::forward::forward(&mut self.store, context, horizon, hook)
    }
}
