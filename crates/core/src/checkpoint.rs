use std::marker::PhantomData;
use std::mem::size_of;

use contime_checkpoints::{ApplyEvents, ApplyWrapper, Snapshot};
use contime_memory::{ConservativeTrackedSize, SizeDelta, TrackedBox, TrackedSizeDelta};

use crate::types::{CheckpointState, CheckpointStorage, CheckpointStorageConfig, History};
use crate::Input;

impl<S> ConservativeTrackedSize for CheckpointState<S>
where
    S: Snapshot + ConservativeTrackedSize,
{
    fn conservative_tracked_size(&self) -> usize {
        let total = self.checkpoints.anchor().map_or(size_of::<Self>(), |anchor| {
            size_of::<Self>().saturating_add(anchor.snapshot.conservative_tracked_size().saturating_sub(size_of::<S>()))
        });
        self.checkpoints.iter().fold(total, |total, checkpoint| {
            total
                .saturating_add(size_of_val(checkpoint).saturating_sub(size_of::<S>()))
                .saturating_add(checkpoint.snapshot.conservative_tracked_size())
        })
    }
}

impl<S> TrackedSizeDelta for CheckpointState<S>
where
    S: Snapshot + ConservativeTrackedSize,
{
    fn size_delta<R>(&mut self, action: impl FnOnce(&mut Self) -> R) -> (R, SizeDelta) {
        let before = self.conservative_tracked_size();
        let result = action(self);
        let after = self.conservative_tracked_size();
        let delta = match after.cmp(&before) {
            std::cmp::Ordering::Greater => SizeDelta::Increase(after - before),
            std::cmp::Ordering::Less => SizeDelta::Decrease(before - after),
            std::cmp::Ordering::Equal => SizeDelta::Unchanged,
        };
        (result, delta)
    }
}

impl<I, S, W> contime_worker::Checkpoints<History<I>> for CheckpointStorage<S, W>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I> + ConservativeTrackedSize,
    W: ApplyWrapper<S, I>,
{
    type Config = CheckpointStorageConfig;
    type Context = W;
    type Time = I::Time;

    fn create(snapshot_id: u128, config: &Self::Config) -> Self {
        let state = CheckpointState { checkpoints: contime_checkpoints::CheckpointStore::new(snapshot_id, config.checkpoints) };
        Self { state: TrackedBox::new(state, config.budget.clone()), wrapper: PhantomData }
    }

    fn update(&mut self, events: &mut History<I>, context: &mut Self::Context) -> Self::Time {
        let affected_from = contime_checkpoints::Events::dirty_time(events).clone();
        self.state.update(|state| {
            contime_checkpoints::replay(&mut state.checkpoints, events, context);
        });
        affected_from
    }

    fn advance_before(&mut self, events: &History<I>, context: &mut Self::Context, horizon: &I::Time) {
        self.state.update(|state| {
            contime_checkpoints::advance_before(&mut state.checkpoints, events, context, horizon);
        });
    }
}

impl<I, S, W> contime_worker::IncrementalCheckpoints<History<I>> for CheckpointStorage<S, W>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I> + ConservativeTrackedSize,
    W: ApplyWrapper<S, I>,
{
    fn invalidate(&mut self, events: &mut History<I>) {
        self.state.update(|state| state.checkpoints.invalidate_from(contime_checkpoints::Events::dirty_time(events)));
        // Insertion changes now belong to the worker's pending-time schedule.
        contime_checkpoints::Events::acknowledge_replay(events);
    }

    fn next_time(&self, events: &History<I>) -> Option<I::Time> {
        self.state.checkpoints.next_replay_time(events)
    }

    fn step(&mut self, events: &History<I>, context: &mut W, target: &I::Time) {
        self.state.update(|state| {
            contime_checkpoints::replay_next(&mut state.checkpoints, events, context, target);
        });
    }
}

impl<I, S, W> contime_worker::QueryCheckpoints<History<I>> for CheckpointStorage<S, W>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I> + ConservativeTrackedSize,
    W: ApplyWrapper<S, I>,
{
    type Context = W;
    type Time = I::Time;
    type Snapshot = S;

    fn query_at(&self, events: &History<I>, context: &mut Self::Context, time: Self::Time) -> Option<Box<Self::Snapshot>> {
        contime_checkpoints::query_at(&self.state.checkpoints, events, context, time)
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use contime_checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
    use contime_memory::ConservativeTrackedSize;
    use contime_worker::Checkpoints as WorkerCheckpoints;
    use criterion::{BatchSize, Criterion};

    use crate::input::prepare_inputs;
    use crate::types::{CheckpointStorage, CheckpointStorageConfig, History};
    use crate::{Input, MemoryBudget};

    struct TestInput {
        id: u128,
        value: usize,
    }

    impl ConservativeTrackedSize for TestInput {
        fn conservative_tracked_size(&self) -> usize {
            32
        }
    }

    impl Input for TestInput {
        type Time = i64;

        fn event_id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> Self::Time {
            self.id as i64
        }

        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }

    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: i64,
        value: usize,
        retained: usize,
    }

    impl ConservativeTrackedSize for TestSnapshot {
        fn conservative_tracked_size(&self) -> usize {
            self.retained
        }
    }

    impl Snapshot for TestSnapshot {
        type Time = i64;

        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    impl ApplyEvents<TestInput> for TestSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestInput) -> Self {
            Self { retained: 64, ..Self::default() }
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, TestInput>) {
            self.value += batch.events.iter().map(|event| event.value).sum::<usize>();
            self.retained += batch.events.len() * 8;
        }
    }

    fn config(budget: MemoryBudget) -> CheckpointStorageConfig {
        CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 100 }, budget }
    }

    fn history(budget: &MemoryBudget, count: u128) -> History<TestInput> {
        let mut history = History::with_horizon(0);
        let events = prepare_inputs(budget, (0..count).map(|id| TestInput { id, value: 1 }).collect()).unwrap();
        for event in events {
            history.insert(event);
        }
        history
    }

    #[test]
    fn checkpoint_replay_materializes_state_acknowledges_history_and_tracks_growth() {
        let budget = MemoryBudget::new(100_000, 1_000);
        let mut history = history(&budget, 2);
        let mut checkpoints =
            <CheckpointStorage<TestSnapshot, ()> as WorkerCheckpoints<History<TestInput>>>::create(7, &config(budget.clone()));
        let before_replay = budget.used();

        let affected_from = checkpoints.update(&mut history, &mut ());

        assert_eq!(affected_from, 0);
        assert_eq!(checkpoints.state.checkpoints.current().unwrap().snapshot.value, 2);
        assert_eq!(contime_checkpoints::Events::dirty_time(&history), &1);
        assert!(budget.used() > before_replay);
        let events_only = {
            drop(checkpoints);
            budget.used()
        };
        drop(history);
        assert!(events_only > 0);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn incremental_checkpoint_invalidation_tracks_growth_and_keeps_pending_suffix() {
        use contime_worker::IncrementalCheckpoints;
        let budget = MemoryBudget::new(100_000, 1_000);
        let mut history = history(&budget, 3);
        let mut checkpoints =
            <CheckpointStorage<TestSnapshot, ()> as WorkerCheckpoints<History<TestInput>>>::create(7, &config(budget.clone()));
        let before = budget.used();
        checkpoints.invalidate(&mut history);
        assert_eq!(contime_checkpoints::Events::dirty_time(&history), &2);
        assert_eq!(checkpoints.next_time(&history), Some(0));
        checkpoints.step(&history, &mut (), &0);
        assert_eq!(checkpoints.state.checkpoints.current().unwrap().snapshot.value, 1);
        assert_eq!(checkpoints.next_time(&history), Some(1));
        assert!(budget.used() > before);
        checkpoints.step(&history, &mut (), &1);
        checkpoints.step(&history, &mut (), &2);
        assert_eq!(checkpoints.next_time(&history), None);
        drop((checkpoints, history));
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn incremental_late_invalidation_releases_stale_checkpoint_memory_before_query() {
        use contime_worker::{IncrementalCheckpoints, QueryCheckpoints};
        let budget = MemoryBudget::new(100_000, 1_000);
        let mut history = History::with_horizon(0);
        for event in prepare_inputs(&budget, [0, 2, 4].map(|id| TestInput { id, value: 1 }).into()).unwrap() {
            history.insert(event);
        }
        let config = CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 1 }, budget: budget.clone() };
        let mut checkpoints = <CheckpointStorage<TestSnapshot, ()> as WorkerCheckpoints<History<TestInput>>>::create(7, &config);
        checkpoints.invalidate(&mut history);
        for time in [0, 2, 4] {
            checkpoints.step(&history, &mut (), &time);
        }
        for event in prepare_inputs(&budget, vec![TestInput { id: 1, value: 10 }]).unwrap() {
            history.insert(event);
        }
        let before = budget.used();
        checkpoints.invalidate(&mut history);
        assert!(budget.used() < before);
        assert_eq!(checkpoints.state.checkpoints.current().unwrap().key.time, 0);
        assert_eq!(checkpoints.next_time(&history), Some(1));
        assert_eq!(checkpoints.query_at(&history, &mut (), 4).unwrap().value, 13);
        assert_eq!(checkpoints.next_time(&history), Some(1));
        drop((checkpoints, history));
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn checkpoint_advancement_tracks_anchor_replacement_and_pruned_checkpoints() {
        let budget = MemoryBudget::new(1_000_000, 1_000);
        let mut history = history(&budget, 100);
        let checkpoint_config = CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 50 }, budget: budget.clone() };
        let mut checkpoints = <CheckpointStorage<TestSnapshot, ()> as WorkerCheckpoints<History<TestInput>>>::create(7, &checkpoint_config);
        checkpoints.update(&mut history, &mut ());
        let before = budget.used();

        WorkerCheckpoints::advance_before(&mut checkpoints, &history, &mut (), &101);

        let anchor = checkpoints.state.checkpoints.anchor().unwrap();
        assert_eq!(anchor.boundary.as_ref().unwrap().time, 99);
        assert!(budget.used() < before);
    }

    #[test]
    fn retention_hook_size_changes_are_accounted_for() {
        struct Compact;
        impl contime_checkpoints::ApplyWrapper<TestSnapshot, TestInput> for Compact {
            fn retain_snapshot(&mut self, snapshot: &mut TestSnapshot, _: &i64) {
                snapshot.retained = 64;
            }
        }
        let plain_budget = MemoryBudget::new(100_000, 1_000);
        let compact_budget = MemoryBudget::new(100_000, 1_000);
        let mut plain_history = history(&plain_budget, 2);
        let mut compact_history = history(&compact_budget, 2);
        let mut plain =
            <CheckpointStorage<TestSnapshot, ()> as WorkerCheckpoints<History<TestInput>>>::create(7, &config(plain_budget.clone()));
        let mut compact =
            <CheckpointStorage<TestSnapshot, Compact> as WorkerCheckpoints<History<TestInput>>>::create(7, &config(compact_budget.clone()));
        plain.update(&mut plain_history, &mut ());
        compact.update(&mut compact_history, &mut Compact);
        assert_eq!(plain_budget.used(), compact_budget.used());

        plain.advance_before(&plain_history, &mut (), &1);
        compact.advance_before(&compact_history, &mut Compact, &1);
        // Anchor T0 retains 72 bytes, tip T1 retains 80. Both compact to 64.
        assert_eq!(plain_budget.used() - compact_budget.used(), 24);
        assert_eq!(compact.state.checkpoints.anchor().unwrap().snapshot.value, 1);
        assert_eq!(compact.state.checkpoints.current().unwrap().snapshot.value, 2);
        drop((plain, compact, plain_history, compact_history));
        assert_eq!((plain_budget.used(), compact_budget.used()), (0, 0));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_checkpoint() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/checkpoint/replay_1000_events", |bencher| {
            bencher.iter_batched(
                || {
                    let budget = MemoryBudget::new(usize::MAX, 0);
                    let history = history(&budget, 1_000);
                    let checkpoints =
                        <CheckpointStorage<TestSnapshot, ()> as WorkerCheckpoints<History<TestInput>>>::create(7, &config(budget));
                    (history, checkpoints)
                },
                |(mut history, mut checkpoints)| {
                    checkpoints.update(&mut history, &mut ());
                    black_box((history, checkpoints))
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
