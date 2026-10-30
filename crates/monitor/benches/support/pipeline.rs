//! Identical prepared workloads for correctness tests and matched benchmarks.
use contime_monitor::{Monitor, Snapshot as Metrics, Totals};
use contime_runtime::{
    checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp},
    AdvanceTime, ConTime, ConTimeConfig, Input, Placement,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Time(pub u64);
impl Timestamp for Time {
    fn minimum() -> Self {
        Self(0)
    }
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool {
        true
    }
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}
#[derive(Clone, Debug)]
pub struct Change {
    pub sequence: u64,
    pub snapshots: usize,
    pub fanout: usize,
}
impl Event for Change {
    type Time = Time;
    fn time(&self) -> Time {
        Time(self.sequence)
    }
}
impl Input for Change {
    fn event_id(&self) -> u128 {
        u128::from(self.sequence)
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for target in 0..self.fanout {
            emit(((self.sequence as usize - 1 + target) % self.snapshots + 1) as u128);
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counter {
    pub id: u128,
    pub time: Time,
    pub count: u64,
    pub hash: u64,
}
fn append(state: &mut Counter, event: &Change) {
    state.count += 1;
    state.hash = state.hash.rotate_left(7) ^ event.sequence.wrapping_mul(0x9e3779b97f4a7c15);
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
impl ApplyEvents<Change> for Counter {
    fn create(id: u128, _event: &Change) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Change>) {
        for event in batch.events {
            append(self, event);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    PerEvent,
    PerBatch,
}
impl Mode {
    pub const ALL: [Self; 3] = [Self::Off, Self::PerEvent, Self::PerBatch];
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "disabled",
            Self::PerEvent => "per-event",
            Self::PerBatch => "per-64-events",
        }
    }
}
pub const BATCH: usize = 64;
pub struct Harness {
    pub runtime: ConTime<Change, Counter, ()>,
    pub events: Vec<Change>,
    pub expected: Vec<Counter>,
    pub fanout: usize,
}
// Metrics use the host wall clock including validation; only `measured` is
// Criterion timing. Metric rates/latency must not be compared to Criterion.
pub struct Report {
    pub measured: Duration,
    pub metrics: Option<Metrics>,
    pub totals: Totals,
}
impl Harness {
    pub fn new(workers: usize, snapshots: usize, events: usize, fanout: usize) -> Self {
        assert!(workers > 0 && fanout > 0 && fanout <= snapshots && events > 0);
        let runtime = ConTime::start(
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
        .unwrap();
        let mut expected: Vec<_> = (1..=snapshots).map(|id| Counter { id: id as u128, ..Counter::default() }).collect();
        for sequence in 1..=events as u64 {
            // Reference indexing is independent of Input::snapshot_ids.
            for offset in 0..fanout {
                let index = (sequence as usize - 1 + offset) % snapshots;
                expected[index].count += 1;
                expected[index].hash = expected[index].hash.rotate_left(7) ^ sequence.wrapping_mul(0x9e3779b97f4a7c15);
            }
        }
        let mut inputs: Vec<_> = (1..=events as u64).map(|sequence| Change { sequence, snapshots, fanout }).collect();
        for chunk in inputs.chunks_mut(256) {
            chunk.reverse();
        }
        Self { runtime, events: inputs, expected, fanout }
    }
    /// Timed submission/settled replay/query plus monitor operations; validation excluded.
    /// Event generation, runtime startup and shutdown are outside this method.
    pub fn run(&mut self, mode: Mode) -> Report {
        let mut monitor = (mode != Mode::Off).then(Monitor::default);
        // Initial sampling/allocation is real monitor work and included.
        let start = Instant::now();
        if let Some(m) = &mut monitor {
            m.sample(Duration::ZERO).unwrap();
        }
        let count = self.events.len();
        let totals = Totals { events: count as u64, applications: (count * self.fanout) as u64 };
        let (responses, inserted) = crossbeam_channel::unbounded();
        for (index, event) in self.events.drain(..).enumerate() {
            self.runtime.send(std::iter::once(event), responses.clone()).unwrap();
            if let Some(m) = &mut monitor {
                if mode == Mode::PerEvent {
                    m.record_submission(Totals { events: 1, applications: self.fanout as u64 }).unwrap();
                } else if (index + 1) % BATCH == 0 || index + 1 == count {
                    let n = if (index + 1) % BATCH == 0 { BATCH } else { (index + 1) % BATCH };
                    m.record_submission(Totals { events: n as u64, applications: (n * self.fanout) as u64 }).unwrap();
                }
            }
        }
        drop(responses);
        assert!(
            inserted.recv_timeout(Duration::from_secs(10)).is_err_and(|e| e == crossbeam_channel::RecvTimeoutError::Disconnected),
            "unexpected rejection or insertion timeout"
        );
        self.runtime.advance_to(Time(count as u64)).unwrap();
        // Controlled benchmark only: settle replay identically in all variants.
        self.runtime.wait_until_idle(Duration::from_secs(10)).unwrap();
        let query = monitor.as_mut().map(|m| m.start_query(start.elapsed()).unwrap());
        let actual = self.runtime.query_at(Time(count as u64), (1..=self.expected.len()).map(|id| id as u128)).unwrap();
        let mut measured = start.elapsed();
        self.validate(&actual);
        // Separate timer excludes validation but includes response bookkeeping.
        let resumed = Instant::now();
        let metrics = monitor.as_mut().map(|m| {
            m.complete_query(query.unwrap(), start.elapsed()).unwrap();
            m.sample(start.elapsed()).unwrap()
        });
        measured += resumed.elapsed();
        Report { measured, metrics, totals }
    }
    pub fn validate(&self, actual: &[Box<Counter>]) {
        let mut seen = vec![false; self.expected.len()];
        for state in actual {
            let index = usize::try_from(state.id).unwrap().checked_sub(1).unwrap();
            assert!(!seen[index], "duplicate snapshot");
            seen[index] = true;
            assert_eq!(state.count, self.expected[index].count);
            assert_eq!(state.hash, self.expected[index].hash, "canonical event order changed");
        }
        for (index, expected) in self.expected.iter().enumerate() {
            assert_eq!(seen[index], expected.count > 0, "missing or unexpected snapshot");
        }
    }
    pub fn shutdown(self) {
        self.runtime.shutdown();
    }
}
