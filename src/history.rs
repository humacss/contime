use std::collections::VecDeque;

use crate::{ApplyEvent, Snapshot};

use super::{apply_event_in_place};

pub trait Deps: Default {
    fn apply_event_in_place<S: Snapshot>(&self, new_event: S::Event, ordered_checkpoints: &mut VecDeque<Checkpoint<S>>, ordered_events: &mut VecDeque<S::Event>,checkpoint_interval: usize) -> isize;
}

#[derive(Default)]
pub struct DefDeps;

impl Deps for DefDeps {
    fn apply_event_in_place<S: Snapshot>(&self, new_event: S::Event, ordered_checkpoints: &mut VecDeque<Checkpoint<S>>, ordered_events: &mut VecDeque<S::Event>,checkpoint_interval: usize) -> isize { 
        apply_event_in_place(new_event, ordered_checkpoints, ordered_events, checkpoint_interval) 
    }
}

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
pub struct LocalSnapshotHistory<S, D>
where
    S: Snapshot,
    D: Deps,
{
    pub snapshot_id: SnapshotId,
    pub ordered_checkpoints: VecDeque<Checkpoint<S>>,
    pub ordered_events: VecDeque<S::Event>,
    pub checkpoint_interval: usize,

    deps: D,
}

const CHECKPOINT_INTERVAL: usize = 100;

impl<S, D> LocalSnapshotHistory<S, D>
where
    S: Snapshot + 'static,
    D: Deps,
{
    pub fn new(snapshot_id: SnapshotId) -> Self {
        let mut ordered_checkpoints = VecDeque::new();
        let ordered_events = VecDeque::new(); 

        ordered_checkpoints.push_back(Checkpoint { snapshot: S::default(), next_event_index: 0, event_count: CHECKPOINT_INTERVAL });
        ordered_checkpoints[0].snapshot.set_time(0);

        ordered_checkpoints.push_back(Checkpoint { snapshot: S::default(), next_event_index: 0, event_count: 0 });
        
        Self { snapshot_id, ordered_checkpoints, ordered_events, checkpoint_interval: CHECKPOINT_INTERVAL, deps: D::default() }
    }

    pub fn apply_event(&mut self, event: S::Event) -> isize {
        if event.snapshot_id() != self.snapshot_id {
            return 0;
        }

        return self.deps.apply_event_in_place(
            event,
            &mut self.ordered_checkpoints,
            &mut self.ordered_events,
            self.checkpoint_interval,
        );
    }
}

pub type SnapshotHistory<S> = LocalSnapshotHistory<S, DefDeps>;

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    use crate::{Event, TestEvent, TestSnapshot};

    #[rstest]
    #[case::empty(1, 2, (2, 3), vec![], vec![], 0, 1)]
    #[case::wrong_snapshot(1, 2, (3, 3), vec![], vec![], 0, 0)]
    fn test_apply_event(
        #[case] bytes_delta: isize,
        #[case] snapshot_id: u128,
        #[case] event: (u128, u128),
        #[case] checkpoints: Vec<i64>,
        #[case] events: Vec<(u128, i64)>,
        #[case] checkpoint_interval: usize,
        #[case] expected: isize,
    ) {
        struct TestDeps {
            new_event: TestEvent,
            snapshot_id: SnapshotId,
            ordered_checkpoints: VecDeque<Checkpoint<TestSnapshot>>,
            ordered_events: VecDeque<TestEvent>,
            checkpoint_interval: usize,
            bytes_delta: isize,
        }

        impl Default for TestDeps {
            fn default() -> Self { 
                Self {
                    new_event: TestEvent::Positive(0, 0, 0, 0),
                    snapshot_id: 0,
                    ordered_checkpoints: VecDeque::new(),
                    ordered_events: VecDeque::new(),
                    checkpoint_interval: 0,
                    bytes_delta: 0,
                }
            }
        }

        impl Deps for TestDeps
         {
            fn apply_event_in_place<S: Snapshot>(&self, new_event: S::Event, ordered_checkpoints: &mut VecDeque<Checkpoint<S>>, ordered_events: &mut VecDeque<S::Event>,checkpoint_interval: usize) -> isize
            {
                assert_eq!(self.new_event.id(), new_event.id(), "wrong new_event input");

                assert_eq!(
                    self.ordered_events.clone().into_iter().map(|event| event.id()).collect::<Vec<u128>>(),
                    ordered_events.into_iter().map(|event| event.id()).collect::<Vec<u128>>(),
                    "wrong ordered_events input"
                );

                assert_eq!(
                    self.ordered_checkpoints.clone().into_iter().map(|checkpoint| (checkpoint.snapshot.id(), checkpoint.snapshot.time())).collect::<Vec<(u128, i64)>>(),
                    ordered_checkpoints.into_iter().map(|checkpoint| (checkpoint.snapshot.id(), checkpoint.snapshot.time())).collect::<Vec<(u128, i64)>>(),
                    "wrong ordered_checkpoints input"
                );
                
                assert_eq!(self.checkpoint_interval, checkpoint_interval, "wrong checkpoint interval input");

                self.bytes_delta
            }
        }

        let mut history = LocalSnapshotHistory::<TestSnapshot, TestDeps>::new(snapshot_id);
        history.ordered_checkpoints = checkpoints.into_iter().map(|time| Checkpoint::<TestSnapshot>::new(TestSnapshot { id: snapshot_id, time, items: vec![], sum: 0 } )).collect();
        history.ordered_events = events.into_iter().map(|(id, time)| TestEvent::Positive(id, time, snapshot_id, 1) ).collect();
        history.checkpoint_interval = checkpoint_interval;

        let new_event = TestEvent::Positive(event.0, 0, event.1, 1);

        history.deps.new_event = new_event.clone();
        history.deps.snapshot_id = snapshot_id;
        history.deps.ordered_checkpoints = history.ordered_checkpoints.clone();
        history.deps.ordered_events = history.ordered_events.clone();
        history.deps.checkpoint_interval = history.checkpoint_interval;
        history.deps.bytes_delta = bytes_delta;
        
        let actual = history.apply_event(new_event);

        assert_eq!(expected, actual, "wrong bytes_delta output");
    }
}