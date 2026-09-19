use std::sync::Arc;
use std::time::Duration;

use contime_core::{checkpoints::*, ConTime, ConTimeConfig, IdleError, Input, Placement, RejectionReason};
use contime_memory::ConservativeTrackedSize;
use crossbeam_channel::{unbounded, Receiver, Sender};

struct Event {
    time: u64,
    id: u128,
    snapshots: Vec<u128>,
}
impl ConservativeTrackedSize for Event {
    fn conservative_tracked_size(&self) -> usize {
        64 + self.snapshots.len() * 16
    }
}
impl Input for Event {
    type Time = u64;
    fn event_id(&self) -> u128 {
        self.id
    }
    fn time(&self) -> u64 {
        self.time
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for &id in &self.snapshots {
            emit(id);
        }
    }
}
fn event(time: u64) -> Event {
    Event { time, id: time.into(), snapshots: vec![1] }
}

#[derive(Clone, Default)]
struct State {
    values: Vec<u64>,
}
impl ConservativeTrackedSize for State {
    fn conservative_tracked_size(&self) -> usize {
        24 + self.values.capacity() * 8
    }
}
impl Snapshot for State {
    type Time = u64;
    fn set_time(&mut self, _: u64) {}
}
impl ApplyEvents<Event> for State {
    fn create(_: u128, _: &Event) -> Self {
        Self::default()
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, u64, Event>) {
        self.values.extend(batch.events.iter().map(|event| event.time));
    }
}

#[derive(Clone)]
struct Observe {
    applied: Sender<u64>,
    block: Option<Arc<(u64, Receiver<()>)>>,
}
impl ApplyWrapper<State, Event> for Observe {
    fn replay_event_batch(&mut self, batch: EventBatch<'_, u64, Event>, inner: &mut ApplyInner<'_, State>) {
        let time = batch.time;
        self.apply_event_batch(batch, inner);
        self.applied.send(time).unwrap();
        if let Some(block) = &self.block {
            if time == block.0 {
                block.1.recv().unwrap();
            }
        }
    }
}

fn config(routers: usize, workers: usize, retention: u64) -> ConTimeConfig<u64> {
    ConTimeConfig {
        router_count: routers,
        worker_count: workers,
        placement: Placement::default(),
        memory_limit: 10_000_000,
        memory_buffer: 1_000,
        history_retention: retention,
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_millis(1),
            replays_per_receive: 1,
            deadline_compaction_minimum: 1024,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: CheckpointConfig { interval: 1 },
    }
}

#[test]
fn future_history_can_be_queried_without_advancing_or_publishing_effects() {
    let (applied, effects) = unbounded();
    let core = ConTime::start(config(2, 2, 1_000), Observe { applied, block: None }).unwrap();
    core.apply([event(10), event(20)]).unwrap();
    core.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(effects.is_empty());
    assert_eq!(core.query_at(20, [1]).unwrap()[0].values, [10, 20]);
    assert!(effects.is_empty());
    core.advance_to(10).unwrap();
    core.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert_eq!(effects.try_iter().collect::<Vec<_>>(), [10]);
    core.advance_to(20).unwrap();
    core.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert_eq!(effects.try_iter().collect::<Vec<_>>(), [20]);
    core.shutdown();
}

#[test]
fn an_active_application_keeps_causal_work_admissible_while_pruning_waits() {
    let (applied, effects) = unbounded();
    let (release, blocked) = unbounded();
    let core = ConTime::start(config(2, 2, 0), Observe { applied, block: Some(Arc::new((10, blocked))) }).unwrap();
    core.apply([event(10), event(20)]).unwrap();
    core.advance_to(100).unwrap();
    assert_eq!(effects.recv_timeout(Duration::from_secs(2)).unwrap(), 10);
    assert_eq!(core.wait_until_idle(Duration::from_millis(20)), Err(IdleError::Timeout));
    core.apply_internal(10, [event(11)]).unwrap();
    core.apply([event(9)]).unwrap();
    let rejection = core.errors().recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(rejection.reason, RejectionReason::BeforeHistoryHorizon);
    release.send(()).unwrap();
    core.wait_until_idle(Duration::from_secs(3)).unwrap();
    assert_eq!(core.query_at(100, [1]).unwrap()[0].values, [10, 11, 20]);
    assert!(core.errors().is_empty());
    core.apply_internal(10, [event(12)]).unwrap();
    assert_eq!(core.errors().recv_timeout(Duration::from_secs(2)).unwrap().reason, RejectionReason::BeforeSafeTime);
    core.shutdown();
}

#[test]
fn accepted_fanout_is_preserved_across_multi_router_pruning() {
    let core = ConTime::<Event, State, ()>::start(config(3, 4, 0), ()).unwrap();
    core.apply((1..=50).map(|time| Event { time, id: time.into(), snapshots: (0..16).collect() })).unwrap();
    core.advance_to(100).unwrap();
    core.wait_until_idle(Duration::from_secs(3)).unwrap();
    let states = core.query_at(100, 0..16).unwrap();
    assert_eq!(states.len(), 16);
    for state in states {
        assert_eq!(state.values, (1..=50).collect::<Vec<_>>());
    }
    assert!(core.errors().is_empty());
    core.shutdown();
}

type Emit = Arc<std::sync::OnceLock<Box<dyn Fn(u64) + Send + Sync>>>;

#[derive(Clone)]
struct Causal(Emit);
impl ApplyWrapper<State, Event> for Causal {
    fn replay_event_batch(&mut self, batch: EventBatch<'_, u64, Event>, inner: &mut ApplyInner<'_, State>) {
        let time = batch.time;
        inner.apply_event_batch(batch);
        (self.0.get().unwrap())(time);
    }
}

#[test]
fn live_callbacks_submit_cross_worker_causal_work_before_their_reports() {
    let emit: Emit = Arc::new(std::sync::OnceLock::new());
    let core = Arc::new(ConTime::start(config(3, 4, 0), Causal(emit.clone())).unwrap());
    let weak = Arc::downgrade(&core);
    assert!(emit
        .set(Box::new(move |time| {
            if time < 20 {
                weak.upgrade()
                    .unwrap()
                    .apply_internal(time, [Event { time: time + 1, id: (time + 1).into(), snapshots: vec![((time + 1) % 4).into()] }])
                    .unwrap();
            }
        }))
        .is_ok());
    core.apply([event(10)]).unwrap();
    core.advance_to(100).unwrap();
    core.wait_until_idle(Duration::from_secs(5)).unwrap();
    let mut values = core.query_at(100, 0..4).unwrap().into_iter().flat_map(|state| state.values).collect::<Vec<_>>();
    values.sort_unstable();
    assert_eq!(values, (10..=20).collect::<Vec<_>>());
    assert!(core.errors().is_empty());
    let core = Arc::try_unwrap(core).ok().unwrap();
    core.shutdown();
}
