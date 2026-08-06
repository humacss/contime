use std::fmt::Debug;

use crate::ContimeTime;

/// A point-in-time state for one logical entity in `contime`.
pub trait Snapshot: Send + Sync + Clone + Debug + PartialEq + Eq {
    /// Ordered time type shared by this snapshot and all of its inputs.
    type Time: ContimeTime;
    /// Unified temporal input type retained by this snapshot history.
    type Input: Input<Time = Self::Time> + Clone + PartialEq + Eq;

    /// Returns the logical snapshot id.
    fn id(&self) -> u128;
    /// Returns the snapshot time.
    fn time(&self) -> Self::Time;
    /// Updates the snapshot time after replay or query.
    fn set_time(&mut self, time: Self::Time);
    /// Returns a conservative upper-bound estimate for memory accounting.
    fn conservative_size(&self) -> u64;
}

/// A globally identified temporal input retained and replayed by `contime`.
pub trait Input: Send + Sync + Debug {
    /// Complete ordered time used by this input.
    type Time: ContimeTime;

    /// Returns the input id used for ordering and duplicate detection.
    fn id(&self) -> u128;
    /// Returns the complete ordered input time.
    fn time(&self) -> Self::Time;
    /// Returns a conservative upper-bound estimate for memory accounting.
    fn conservative_size(&self) -> u64;
}

/// A temporal input with snapshot-application behavior.
pub trait Event: Input {}

/// A temporal input interpreted by an apply wrapper rather than a snapshot.
pub trait Marker: Input {}

/// Routes one concrete event to a snapshot and initializes that snapshot's identity.
pub trait SnapshotEvent<S>: Event<Time = S::Time>
where
    S: Snapshot,
{
    /// Returns the logical snapshot id affected by this event.
    fn snapshot_id(&self) -> u128;

    /// Initializes only the identity fields of a clean default snapshot.
    fn set_snapshot_identity(&self, snapshot: &mut S);
}

/// A generated or user-defined snapshot lane universe.
pub trait SnapshotLanes: Snapshot {
    /// Materializes the snapshot lane selected by `snapshot_id` from an applicable event input.
    fn materialize(snapshot_id: u128, input: &Self::Input) -> Option<Self>;

    /// Returns the generated in-memory index of this snapshot lane variant.
    fn lane_index(&self) -> usize;

    /// Returns the snapshot lane variant selected by an event input and snapshot id.
    fn input_lane_index(snapshot_id: u128, input: &Self::Input) -> Option<usize>;
}

/// One same-complete-time batch of temporal inputs routed to a snapshot.
#[derive(Debug)]
pub struct InputBatch<'a, I: Input> {
    pub snapshot_id: u128,
    pub time: I::Time,
    pub inputs: &'a [&'a I],
}

impl<'a, I: Input> Clone for InputBatch<'a, I> {
    fn clone(&self) -> Self {
        Self { snapshot_id: self.snapshot_id, time: self.time.clone(), inputs: self.inputs }
    }
}

/// Applies one bucket of events with the same complete ordered time to a snapshot.
#[derive(Debug)]
pub struct ApplyBatch<'a, E: Event> {
    pub snapshot_id: u128,
    pub time: E::Time,
    pub events: &'a [&'a E],
}

impl<'a, E: Event> Clone for ApplyBatch<'a, E> {
    fn clone(&self) -> Self {
        Self { snapshot_id: self.snapshot_id, time: self.time.clone(), events: self.events }
    }
}

/// Applies one concrete event type to a snapshot.
pub trait ApplyEvents<E>: Snapshot
where
    E: Event<Time = Self::Time>,
{
    fn apply_events(&mut self, batch: ApplyBatch<'_, E>);
}

/// Routes a plain marker through a generated snapshot lane universe.
pub trait InputRoute {
    fn snapshot_ids(&self) -> Vec<u128>;
}

/// The generated union of every temporal input accepted by one ConTime instance.
pub trait InputLanes<SL: Snapshot<Input = Self>>: Input<Time = SL::Time> + Clone + PartialEq + Eq {
    /// Returns the snapshot ids whose histories should retain this input.
    fn snapshot_ids(&self) -> Vec<u128>;

    /// Returns whether this input has snapshot-application behavior.
    fn is_event(&self) -> bool;

    /// Applies the concrete event variants in `batch` to `snapshot`.
    fn apply_events(snapshot: &mut SL, batch: InputBatch<'_, Self>);
}

impl<S, E> InputLanes<S> for E
where
    S: Snapshot<Input = E> + ApplyEvents<E> + Default,
    E: Event<Time = S::Time> + SnapshotEvent<S> + Clone + PartialEq + Eq,
{
    fn snapshot_ids(&self) -> Vec<u128> {
        vec![self.snapshot_id()]
    }

    fn is_event(&self) -> bool {
        true
    }

    fn apply_events(snapshot: &mut S, batch: InputBatch<'_, Self>) {
        <S as ApplyEvents<E>>::apply_events(
            snapshot,
            ApplyBatch { snapshot_id: batch.snapshot_id, time: batch.time, events: batch.inputs },
        );
    }
}
