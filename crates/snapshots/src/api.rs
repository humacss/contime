//! Worker-facing access to one snapshot's checkpoint history.
use crate::{Apply, Checkpoint, Event, EventStore, ForwardError, Insert, InsertEventStore, NoCheckpoint, Snapshot, Store, Timestamp};

/// Owns event history and checkpoints without exposing playback or commit policies.
pub struct SnapshotStore<S: Snapshot, H> {
    store: Store<S, H>,
}

impl<S: Snapshot, H> SnapshotStore<S, H> {
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
    /// or apply the snapshot; None means no remaining event application.
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
    pub fn insert(&mut self, event: H::Event) -> Insert
    where
        H: InsertEventStore,
    {
        self.store.insert(event)
    }

    /// Physically removes history before the current horizon, except for its
    /// predecessor checkpoint. Can be called independently of forwarding frequency.
    pub fn prune(&mut self) {
        self.store.prune();
    }

    /// Reconstructs state through the inclusive target without retaining changes.
    /// Supply an effect-free context for query application.
    pub fn query<C>(&mut self, time: S::Time, context: &C) -> Result<Box<S>, NoCheckpoint>
    where
        H::Event: Apply<Checkpoint<S>, C>,
    {
        crate::query::query_at(&mut self.store, context, time)
    }

    /// Ensures retained state incorporates accepted events through the inclusive target.
    /// Resumes from valid state; replay and checkpoint placement are internal details.
    pub fn process_until<C>(&mut self, time: S::Time, context: &C) -> Result<(), NoCheckpoint>
    where
        H::Event: Apply<Checkpoint<S>, C>,
    {
        crate::replay::replay(&mut self.store, context, time)
    }

    /// Forwards through the horizon's predecessor, then publishes the horizon.
    /// The hook may compact the snapshot while preserving observable state.
    /// Events and obsolete checkpoints are not physically removed.
    pub fn forward<C>(&mut self, horizon: S::Time, context: &C, hook: impl FnMut(&mut S, &S::Time, &C)) -> Result<(), ForwardError>
    where
        S::Time: Timestamp,
        H::Event: Apply<Checkpoint<S>, C>,
    {
        crate::forward::forward(&mut self.store, context, horizon, hook)
    }
}
