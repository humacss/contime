use std::collections::VecDeque;

use crate::{ApplyEvent, Snapshot};

use super::{apply_event_in_place};

type SnapshotId = u128;

#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint<S>
where
    S: Snapshot,
{
    pub snapshot: S,
    pub next_event_index: usize,
    pub event_count: usize,
}

impl<S> Checkpoint<S>
where
    S: Snapshot,
{
    pub fn new(snapshot: S) -> Self {
        Self { snapshot, next_event_index: 0, event_count: 0 }
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotHistory<S>
where
    S: Snapshot,
{
    pub snapshot_id: SnapshotId,
    pub ordered_checkpoints: VecDeque<Checkpoint<S>>,
    pub ordered_events: VecDeque<S::Event>,
    pub max_event_count: usize,
    pub checkpoint_interval: usize,
}

impl<S> SnapshotHistory<S>
where
    S: Snapshot + 'static,
{
    pub fn new(snapshot_id: SnapshotId) -> Self {
        let mut ordered_checkpoints = VecDeque::new();
        let ordered_events = VecDeque::new(); 

        // make sure snapshot time is always 0
        ordered_checkpoints.push_back(Checkpoint { snapshot: S::default(), next_event_index: 0, event_count: 100 });
        ordered_checkpoints.push_back(Checkpoint { snapshot: S::default(), next_event_index: 0, event_count: 0 });
        
        Self { snapshot_id, ordered_checkpoints, ordered_events, checkpoint_interval: 100, max_event_count: 1000 }
    }

    pub fn apply_event(&mut self, event: S::Event) {
        if event.snapshot_id() != self.snapshot_id {
            return;
        }

        apply_event_in_place(
            event,
            &mut self.ordered_checkpoints,
            &mut self.ordered_events,
            self.checkpoint_interval,
        );
    }
}
