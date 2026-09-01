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
