//! Regression coverage for a reported work timestamp overtaking a replay prefix.
//! Real admission, routers, workers and snapshot/event stores; no Runtime/game.
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use contime_core::{checkpoints::*, ConTime, ConTimeConfig, Input, Placement, RejectionReason};
use crossbeam_channel::{unbounded, Receiver, Sender};

const SOURCE: u128 = 1;
const PREFIX: u64 = 10;
const INITIAL_TARGET: u64 = 100;
const FINAL_TARGET: u64 = 180;
const CUTOFF: u64 = 80;
const TIMEOUT: Duration = Duration::from_secs(5);
const TRIALS: usize = 64;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}
struct TestEvent {
    id: u128,
    time: Time,
    destinations: Vec<u128>,
    value: u64,
}
impl contime_checkpoints::Event for TestEvent {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl Input for TestEvent {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for &id in &self.destinations {
            emit(id);
        }
    }
}
#[derive(Clone, Default)]
struct TestSnapshot {
    id: u128,
    time: Time,
    sum: u64,
}
impl Snapshot for TestSnapshot {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl ApplyEvents<TestEvent> for TestSnapshot {
    fn create(id: u128, _: &TestEvent) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, TestEvent>) {
        self.sum += batch.events.map(|event| event.value).sum::<u64>();
    }
}
type TestCore = ConTime<TestEvent, TestSnapshot, Observe>;
struct Feedback {
    core: Weak<TestCore>,
    time: Time,
    destinations: Vec<u128>,
    gate: Option<(Sender<()>, Receiver<()>)>,
}
#[derive(Clone)]
struct Observe {
    published: Sender<Time>,
    feedback: Option<Arc<OnceLock<Feedback>>>,
}
impl ApplyWrapper<TestSnapshot, TestEvent> for Observe {
    fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, Time, TestEvent>, inner: &mut ApplyInner<'_, TestSnapshot>) {
        let (id, time) = (batch.snapshot_id, batch.time);
        inner.apply_event_batch(batch);
        if id == SOURCE {
            self.published.send(time).unwrap();
            if time == Time(PREFIX) {
                if let Some(feedback) = self.feedback.as_ref().and_then(|slot| slot.get()) {
                    if let Some((entered, released)) = &feedback.gate {
                        entered.send(()).unwrap();
                        released.recv_timeout(TIMEOUT).unwrap();
                    }
                    feedback
                        .core
                        .upgrade()
                        .unwrap()
                        .apply_internal(
                            time,
                            [TestEvent { id: 1_000, time: feedback.time, destinations: feedback.destinations.clone(), value: 7 }],
                        )
                        .unwrap();
                }
            }
        }
    }
}
fn config(routers: usize, workers: usize, interval: u64, retention: u64) -> ConTimeConfig<Time> {
    ConTimeConfig {
        router_count: routers,
        worker_count: workers,
        placement: Placement::default(),
        history_retention: Time(retention),
        pruning_interval: Duration::ZERO,
        checkpoints: CheckpointConfig { interval },
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_millis(1),
            replays_per_receive: 1,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        },
    }
}
fn event(id: u128, time: u64) -> TestEvent {
    TestEvent { id, time: Time(time), destinations: vec![SOURCE], value: 1 }
}

#[test]
fn late_insertion_can_publish_from_before_its_own_timestamp() {
    let expected_initial = [Time(PREFIX), Time(INITIAL_TARGET)];
    let expected_replay = [Time(PREFIX), Time(90), Time(INITIAL_TARGET)];
    let expected_sum = 3;

    let (published, publications) = unbounded();
    let core = ConTime::start(config(1, 1, 100, 1_000), Observe { published, feedback: None }).unwrap();
    core.apply([event(1, PREFIX), event(2, INITIAL_TARGET)]).unwrap();
    core.advance_to(Time(INITIAL_TARGET)).unwrap();
    core.wait_until_idle(TIMEOUT).unwrap();
    let initial: Vec<_> = publications.try_iter().collect();

    core.apply([event(3, 90)]).unwrap();
    core.wait_until_idle(TIMEOUT).unwrap();
    let replay: Vec<_> = publications.try_iter().collect();
    let states = core.query_at(Time(INITIAL_TARGET), [SOURCE]).unwrap();
    let errors: Vec<_> = core.errors().try_iter().collect();
    core.shutdown();

    assert_eq!(initial, expected_initial);
    assert_eq!(replay, expected_replay);
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].sum, expected_sum);
    assert!(errors.is_empty(), "{errors:?}");
}

// Bounded scheduling stress, not a deterministic interleaving guarantee.
// Every trial must succeed; retries never turn a failure into a passing test.
#[rstest::rstest]
#[case::sparse_single_worker(1, 1, 100, 100, 90, 10, vec![2])]
#[case::unbounded_checkpoints(1, 1, 0, 100, 90, 10, vec![2])]
#[case::existing_timestamp(1, 1, 100, 100, 100, 10, vec![2])]
#[case::later_causal_output(1, 1, 100, 100, 90, 20, vec![2])]
#[case::cross_worker(3, 4, 100, 100, 90, 10, vec![2])]
#[case::fanout(3, 4, 100, 100, 90, 10, vec![2, 3, 4])]
#[case::dense_checkpoint_control(1, 1, 1, 100, 90, 10, vec![2])]
#[case::no_pruning_control(3, 4, 100, 1_000, 90, 10, vec![2, 3, 4])]
fn accepted_replay_feedback_survives_horizon_advancement(
    #[case] routers: usize,
    #[case] workers: usize,
    #[case] interval: u64,
    #[case] retention: u64,
    #[case] inserted_time: u64,
    #[case] output_time: u64,
    #[case] destinations: Vec<u128>,
) {
    let expected_source_sum = 3;
    let expected_destination_sum = 7;
    let expected_horizon = Time(FINAL_TARGET.saturating_sub(retention));
    for trial in 0..TRIALS {
        let (published, publications) = unbounded();
        let feedback = Arc::new(OnceLock::new());
        let core = Arc::new(
            ConTime::start(config(routers, workers, interval, retention), Observe { published, feedback: Some(feedback.clone()) }).unwrap(),
        );
        assert!(feedback
            .set(Feedback { core: Arc::downgrade(&core), time: Time(output_time), destinations: destinations.clone(), gate: None })
            .is_ok());
        let horizons = core.subscribe_pruned_horizon().unwrap();
        core.apply([event(1, PREFIX), event(2, INITIAL_TARGET)]).unwrap();
        core.advance_to(Time(INITIAL_TARGET)).unwrap();
        core.wait_until_idle(TIMEOUT).unwrap();
        assert!(core.errors().is_empty(), "initial work failed");
        let initial_publications: Vec<_> = publications.try_iter().collect();
        assert_eq!(initial_publications, [Time(PREFIX), Time(INITIAL_TARGET)]);

        core.apply([event(3, inserted_time)]).unwrap();
        core.advance_to(Time(FINAL_TARGET)).unwrap();
        let idle = core.wait_until_idle(TIMEOUT);
        let errors: Vec<_> = core.errors().try_iter().collect();
        let replay: Vec<_> = publications.try_iter().collect();
        let completed = horizons.try_iter().last();
        let states = core.query_at(Time(FINAL_TARGET), std::iter::once(SOURCE).chain(destinations.iter().copied())).unwrap();
        Arc::try_unwrap(core).ok().unwrap().shutdown();

        assert!(idle.is_ok(), "trial {trial}: idle={idle:?}, rejections={errors:?}, publications={replay:?}");
        assert!(errors.is_empty(), "trial {trial}: rejections={errors:?}, publications={replay:?}, completed={completed:?}");
        assert_eq!(completed, Some(expected_horizon), "trial {trial}");
        assert_eq!(states.len(), destinations.len() + 1);
        for state in states {
            assert_eq!(
                state.sum,
                if state.id == SOURCE { expected_source_sum } else { expected_destination_sum },
                "trial {trial}, snapshot {}",
                state.id
            );
        }
    }
}

#[test]
fn requested_horizon_closes_external_admission_without_cutting_off_active_feedback() {
    let expected_rejection = RejectionReason::BeforeHistoryHorizon;
    let expected_horizon = Time(CUTOFF);
    let expected_destination_sum = 7;

    let (published, _publications) = unbounded();
    let (entered, entering) = unbounded();
    let (release, released) = unbounded();
    let feedback = Arc::new(OnceLock::new());
    let core = Arc::new(ConTime::start(config(3, 4, 100, 100), Observe { published, feedback: Some(feedback.clone()) }).unwrap());
    assert!(feedback
        .set(Feedback { core: Arc::downgrade(&core), time: Time(PREFIX), destinations: vec![2, 3], gate: Some((entered, released)) })
        .is_ok());
    let horizons = core.subscribe_pruned_horizon().unwrap();
    core.apply([event(1, PREFIX)]).unwrap();
    core.advance_to(Time(INITIAL_TARGET)).unwrap();
    entering.recv_timeout(TIMEOUT).unwrap();

    core.advance_to(Time(FINAL_TARGET)).unwrap();
    core.apply([event(2, CUTOFF - 1)]).unwrap();
    // The rejection acknowledges that the coordinator has closed admission.
    // The callback remains blocked, so it has not submitted its causal output yet.
    let rejection = core.errors().recv_timeout(TIMEOUT);
    let while_active: Vec<_> = horizons.try_iter().collect();
    release.send(()).unwrap();
    let idle = core.wait_until_idle(TIMEOUT);
    let errors: Vec<_> = core.errors().try_iter().collect();
    let completed = horizons.try_iter().last();
    let states = core.query_at(Time(FINAL_TARGET), [2, 3]).unwrap();
    Arc::try_unwrap(core).ok().unwrap().shutdown();

    let rejection = rejection.unwrap();
    assert_eq!(rejection.event_id, 2);
    assert_eq!(rejection.reason, expected_rejection);
    assert!(while_active.iter().all(|time| *time < expected_horizon));
    assert!(idle.is_ok(), "{idle:?}");
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(completed, Some(expected_horizon));
    assert_eq!(states.len(), 2);
    assert!(states.iter().all(|state| state.sum == expected_destination_sum));
}

#[test]
fn completed_forwarding_prevents_replay_before_horizon_but_accepts_at_horizon() {
    let expected_sum = 3;
    let expected_publications = [Time(CUTOFF), Time(INITIAL_TARGET)];
    let expected_rejection = RejectionReason::BeforeHistoryHorizon;

    let (published, publications) = unbounded();
    let core = ConTime::start(config(3, 4, 100, 100), Observe { published, feedback: None }).unwrap();
    let horizons = core.subscribe_pruned_horizon().unwrap();
    core.apply([event(1, PREFIX), event(2, INITIAL_TARGET)]).unwrap();
    core.advance_to(Time(FINAL_TARGET)).unwrap();
    core.wait_until_idle(TIMEOUT).unwrap();
    assert_eq!(horizons.try_iter().last(), Some(Time(CUTOFF)));
    assert!(core.errors().is_empty());
    publications.try_iter().for_each(drop);

    core.apply([event(3, CUTOFF), event(4, CUTOFF - 1)]).unwrap();
    core.wait_until_idle(TIMEOUT).unwrap();
    let errors: Vec<_> = core.errors().try_iter().collect();
    let replay: Vec<_> = publications.try_iter().collect();
    let states = core.query_at(Time(INITIAL_TARGET), [SOURCE]).unwrap();
    let expired = core.query_at(Time(CUTOFF - 1), [SOURCE]).unwrap();
    core.shutdown();

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].event_id, 4);
    assert_eq!(errors[0].reason, expected_rejection);
    assert_eq!(replay, expected_publications);
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].sum, expected_sum);
    assert!(expired.is_empty());
}
