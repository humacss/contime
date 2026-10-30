use contime::checkpoints::{self, Snapshot};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Time(i64);
impl checkpoints::Timestamp for Time {
    fn minimum() -> Self {
        Self(i64::MIN)
    }
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime::AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool {
        self.0 >= 0
    }
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}
struct Change {
    id: u128,
    time: Time,
    value: u64,
}
impl checkpoints::Event for Change {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl contime::Input for Change {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(1);
    }
}
#[derive(Clone, Default)]
struct Counter {
    time: Time,
    sum: u64,
}
impl Snapshot for Counter {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl checkpoints::ApplyEvents<Change> for Counter {
    fn create(_: u128, _: &Change) -> Self {
        Self::default()
    }
    fn apply_events(&mut self, batch: checkpoints::ApplyBatch<'_, '_, Time, Change>) {
        self.sum += batch.events.map(|event| event.value).sum::<u64>();
    }
}
#[cfg(feature = "manual")]
#[test]
fn manual_accepts_negative_and_minimum_timestamps_and_prunes() {
    use contime::manual::{ManualConfig, RawManualConTime};
    use std::{cell::RefCell, num::NonZeroUsize, rc::Rc};
    let mut engine = RawManualConTime::<Change, Counter>::new(
        ManualConfig { queue_capacity: NonZeroUsize::new(16).unwrap(), checkpoints: checkpoints::CheckpointConfig { interval: 1 } },
        (),
    );
    for (id, time, value) in [(1, i64::MIN, 1), (2, -10, 2), (3, -5, 4)] {
        engine.apply([Change { id, time: Time(time), value }]).unwrap();
    }
    engine.advance_to(Time(-4)).unwrap();
    while engine.process().unwrap() {}
    let result = Rc::new(RefCell::new(None));
    let out = result.clone();
    engine.query_at(Time(-5), [1], move |states| *out.borrow_mut() = states.first().map(|s| s.sum)).unwrap();
    while engine.process().unwrap() {}
    assert_eq!(*result.borrow(), Some(7));
    engine.prune_before(Time(-7)).unwrap();
    while engine.process().unwrap() {}
    engine.apply([Change { id: 4, time: Time(-8), value: 8 }]).unwrap();
    while engine.process().unwrap() {}
    assert_eq!(engine.take_rejections().len(), 1);
    assert_eq!(engine.worker_status().admission_horizon, Time(-7));
}
#[cfg(feature = "runtime")]
#[test]
fn runtime_accepts_negative_and_minimum_timestamps_and_retains_negative_history() {
    use std::time::Duration;
    let engine = contime::RawConTime::<Change, Counter, ()>::start(
        contime::ConTimeConfig {
            router_count: 1,
            worker_count: 1,
            placement: contime::Placement::default(),
            checkpoints: checkpoints::CheckpointConfig { interval: 1 },
            history_retention: Time(3),
            pruning_interval: Duration::from_millis(1),
        },
        (),
    )
    .unwrap();
    let horizons = engine.subscribe_pruned_horizon().unwrap();
    assert_eq!(horizons.recv_timeout(Duration::from_secs(2)).unwrap(), Time(i64::MIN));
    engine.apply([Change { id: 1, time: Time(i64::MIN), value: 1 }, Change { id: 2, time: Time(-5), value: 2 }]).unwrap();
    engine.advance_to(Time(-4)).unwrap();
    engine.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert_eq!(engine.query_at(Time(-5), [1]).unwrap().first().map(|s| s.sum), Some(3));
    assert_eq!(horizons.recv_timeout(Duration::from_secs(2)).unwrap(), Time(-7));
    engine.shutdown();
}

#[cfg(feature = "runtime")]
#[test]
fn runtime_rejects_negative_retention_before_starting() {
    use std::time::Duration;
    let result = contime::RawConTime::<Change, Counter, ()>::start(
        contime::ConTimeConfig {
            router_count: 1,
            worker_count: 1,
            placement: contime::Placement::default(),
            checkpoints: checkpoints::CheckpointConfig { interval: 1 },
            history_retention: Time(-1),
            pruning_interval: Duration::from_millis(1),
        },
        (),
    );
    match result {
        Ok(engine) => {
            engine.shutdown();
            panic!("negative retention accepted");
        }
        Err(error) => assert_eq!(error.to_string(), "history retention must be nonnegative"),
    }
}
