//! Minimal fixed-payload counter fixture, without simulator or hash work.
use contime_runtime::{
    checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp},
    AdvanceTime, ConTime, ConTimeConfig, Input, Placement,
};
use std::time::Duration;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Time(pub u64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl AdvanceTime for Time {
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}
#[derive(Clone)]
pub struct Tick {
    pub sequence: u64,
    pub id: u128,
}
impl Event for Tick {
    type Time = Time;
    fn time(&self) -> Time {
        Time(self.sequence)
    }
}
impl Input for Tick {
    fn event_id(&self) -> u128 {
        self.sequence.into()
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.id);
    }
}
#[derive(Clone, Default)]
pub struct Counter {
    pub count: u64,
    pub time: Time,
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
impl ApplyEvents<Tick> for Counter {
    fn create(_: u128, _: &Tick) -> Self {
        Self::default()
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Tick>) {
        self.count += batch.events.count() as u64;
    }
}
pub fn start(workers: usize) -> ConTime<Tick, Counter, ()> {
    ConTime::start(
        ConTimeConfig {
            worker_count: workers,
            router_count: 1,
            placement: Placement::default(),
            history_retention: Time(u64::MAX),
            pruning_interval: Duration::from_secs(60),
            checkpoints: CheckpointConfig { interval: 128 },
        },
        (),
    )
    .unwrap()
}
pub fn send(runtime: &ConTime<Tick, Counter, ()>, from: u64, count: u64, snapshots: u64) {
    let (sender, receiver) = crossbeam_channel::unbounded();
    for sequence in from..from + count {
        runtime.send(std::iter::once(Tick { sequence, id: u128::from(sequence % snapshots + 1) }), sender.clone()).unwrap();
    }
    drop(sender);
    assert!(receiver.recv_timeout(Duration::from_secs(10)).is_err_and(|error| error == crossbeam_channel::RecvTimeoutError::Disconnected));
}
