use ahash::AHashMap;

use crate::checkpoints::update_snapshot;
use crate::schedule::Schedule;
use crate::types::{AdvanceTime, Checkpoints, Completion, Events, SnapshotSlot};

pub(crate) fn advance_worker<I, S, K, C, R>(
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    schedule: &mut Schedule,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
    current_time: &mut S::Time,
    horizon: &mut S::Time,
    retention: &S::Time,
    target_time: S::Time,
    completion: impl Sized,
) where
    S: Events<I, Rejection = R>,
    K: Checkpoints<S, Time = S::Time>,
    C: Completion<R>,
{
    if target_time <= *current_time {
        drop(completion);
        return;
    }

    *current_time = target_time;
    *horizon = current_time.saturating_sub(retention);

    let replay_ids = snapshots
        .iter()
        .filter_map(|(snapshot_id, slot)| (schedule.is_dirty(*snapshot_id) && slot.events.dirty_time() < horizon).then_some(*snapshot_id))
        .collect::<Vec<_>>();
    for snapshot_id in replay_ids {
        schedule.take(snapshot_id);
        update_snapshot(snapshot_id, snapshots, checkpoints_config, checkpoints_context);
    }

    for slot in snapshots.values_mut() {
        if let Some(checkpoints) = slot.checkpoints.as_mut() {
            checkpoints.advance_before(&slot.events, checkpoints_context, horizon);
        }
        slot.events.prune_before(horizon);
    }

    drop(completion);
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use ahash::AHashMap;
    use criterion::{BatchSize, Criterion, Throughput};
    use crossbeam_channel::{unbounded, TryRecvError};

    use crate::schedule::Schedule;
    use crate::types::SnapshotSlot;
    use crate::{Checkpoints, EventInsert, Events};

    struct TestInput;

    struct TestEvents {
        dirty_time: i64,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Events<TestInput> for TestEvents {
        type Config = (i64, Arc<Mutex<Vec<&'static str>>>);
        type Rejection = ();
        type Time = i64;

        fn create(_snapshot_id: u128, config: &Self::Config, _horizon: &Self::Time) -> Self {
            Self { dirty_time: config.0, log: Arc::clone(&config.1) }
        }

        fn insert(&mut self, _input: TestInput) -> EventInsert<Self::Rejection> {
            EventInsert { changed: true, rejections: Vec::new() }
        }

        fn dirty_time(&self) -> &Self::Time {
            &self.dirty_time
        }

        fn prune_before(&mut self, _horizon: &Self::Time) {
            self.log.lock().unwrap().push("events");
        }
    }

    struct TestCheckpoints {
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Checkpoints<TestEvents> for TestCheckpoints {
        type Config = Arc<Mutex<Vec<&'static str>>>;
        type Context = ();
        type Time = i64;

        fn create(_snapshot_id: u128, config: &Self::Config) -> Self {
            Self { log: Arc::clone(config) }
        }

        fn update(&mut self, _events: &mut TestEvents, _context: &mut Self::Context) {
            self.log.lock().unwrap().push("replay");
        }

        fn advance_before(&mut self, _events: &TestEvents, _context: &mut Self::Context, _horizon: &Self::Time) {
            self.log.lock().unwrap().push("anchor");
        }
    }

    fn fixture(
        dirty_time: i64,
        scheduled: bool,
    ) -> (
        AHashMap<u128, SnapshotSlot<TestEvents, TestCheckpoints, crossbeam_channel::Sender<Vec<()>>, ()>>,
        Schedule,
        Arc<Mutex<Vec<&'static str>>>,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut snapshots = AHashMap::new();
        snapshots.insert(
            7,
            SnapshotSlot {
                events: TestEvents { dirty_time, log: Arc::clone(&log) },
                checkpoints: Some(TestCheckpoints { log: Arc::clone(&log) }),
                waiters: Vec::new(),
            },
        );
        let mut schedule = Schedule::new(usize::MAX, 2);
        if scheduled {
            schedule.mark_dirty(7, Instant::now());
        }
        (snapshots, schedule, log)
    }

    #[test]
    fn advance_replays_before_anchor_and_event_pruning() {
        let (mut snapshots, mut schedule, log) = fixture(5, true);
        let mut current_time = 0;
        let mut horizon = 0;
        let (completion, done) = unbounded::<()>();

        super::advance_worker::<TestInput, _, _, _, _>(
            &mut snapshots,
            &mut schedule,
            &Arc::clone(&log),
            &mut (),
            &mut current_time,
            &mut horizon,
            &10,
            100,
            completion,
        );

        assert_eq!(*log.lock().unwrap(), vec!["replay", "anchor", "events"]);
        assert_eq!(current_time, 100);
        assert_eq!(horizon, 90);
        assert_eq!(done.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn dirty_state_at_the_horizon_remains_scheduled() {
        let (mut snapshots, mut schedule, _log) = fixture(10, true);
        let mut current_time = 0;
        let mut horizon = 0;

        super::advance_worker::<TestInput, _, _, _, _>(
            &mut snapshots,
            &mut schedule,
            &Arc::new(Mutex::new(Vec::new())),
            &mut (),
            &mut current_time,
            &mut horizon,
            &10,
            20,
            (),
        );

        assert!(schedule.take(7));
    }

    #[test]
    fn older_advancement_is_a_successful_no_op() {
        let (mut snapshots, mut schedule, log) = fixture(5, true);
        let mut current_time = 100;
        let mut horizon = 90;

        super::advance_worker::<TestInput, _, _, _, _>(
            &mut snapshots,
            &mut schedule,
            &Arc::clone(&log),
            &mut (),
            &mut current_time,
            &mut horizon,
            &10,
            90,
            (),
        );

        assert!(log.lock().unwrap().is_empty());
        assert_eq!(current_time, 100);
        assert_eq!(horizon, 90);
    }

    struct BenchEvents {
        dirty_time: i64,
    }

    impl Events<TestInput> for BenchEvents {
        type Config = ();
        type Rejection = ();
        type Time = i64;

        fn create(_snapshot_id: u128, _config: &(), horizon: &i64) -> Self {
            Self { dirty_time: *horizon }
        }

        fn insert(&mut self, _input: TestInput) -> EventInsert<()> {
            EventInsert { changed: true, rejections: Vec::new() }
        }

        fn dirty_time(&self) -> &i64 {
            &self.dirty_time
        }

        fn prune_before(&mut self, horizon: &i64) {
            black_box(horizon);
        }
    }

    struct BenchCheckpoints;

    impl Checkpoints<BenchEvents> for BenchCheckpoints {
        type Config = ();
        type Context = usize;
        type Time = i64;

        fn create(_snapshot_id: u128, _config: &()) -> Self {
            Self
        }

        fn update(&mut self, _events: &mut BenchEvents, context: &mut usize) {
            *context += 1;
        }

        fn advance_before(&mut self, _events: &BenchEvents, context: &mut usize, _horizon: &i64) {
            *context += 1;
        }
    }

    type BenchSnapshots = AHashMap<u128, SnapshotSlot<BenchEvents, BenchCheckpoints, crossbeam_channel::Sender<Vec<()>>, ()>>;

    fn benchmark_fixture(with_checkpoints: bool, dirty: bool) -> (BenchSnapshots, Schedule) {
        let mut snapshots = AHashMap::with_capacity(1_000);
        let mut schedule = Schedule::new(usize::MAX, 2);
        let now = Instant::now();
        for snapshot_id in 0..1_000_u128 {
            snapshots.insert(
                snapshot_id,
                SnapshotSlot {
                    events: BenchEvents { dirty_time: if dirty { 1 } else { 100 } },
                    checkpoints: with_checkpoints.then_some(BenchCheckpoints),
                    waiters: Vec::new(),
                },
            );
            if dirty {
                schedule.mark_dirty(snapshot_id, now);
            }
        }
        (snapshots, schedule)
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_advance() {
        let mut criterion = Criterion::default();
        let mut group = criterion.benchmark_group("worker/advance/1000_histories");
        group.throughput(Throughput::Elements(1_000));
        for (name, with_checkpoints, dirty) in [("clean", false, false), ("anchor_pruning", true, false), ("forced_replay", true, true)] {
            group.bench_function(name, |bencher| {
                bencher.iter_batched(
                    || benchmark_fixture(with_checkpoints, dirty),
                    |(mut snapshots, mut schedule)| {
                        let mut current_time = 0;
                        let mut horizon = 0;
                        let mut context = 0;
                        super::advance_worker::<TestInput, _, _, _, _>(
                            &mut snapshots,
                            &mut schedule,
                            &(),
                            &mut context,
                            &mut current_time,
                            &mut horizon,
                            &10,
                            100,
                            (),
                        );
                        black_box((snapshots, schedule, context));
                    },
                    BatchSize::LargeInput,
                );
            });
        }
        group.finish();
        criterion.final_summary();
    }
}
