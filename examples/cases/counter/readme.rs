use std::thread;
use std::time::{Duration, Instant};

use contime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime::{AdvanceTime, ConTime, ConTimeConfig, Input, Placement};

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct Time(u64);

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
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}

struct CounterChanged {
    event_id: u128,
    counter_ids: Vec<u128>,
    time: Time,
    delta: i64,
}

impl Event for CounterChanged {
    type Time = Time;

    fn time(&self) -> Time {
        self.time
    }
}

impl Input for CounterChanged {
    fn event_id(&self) -> u128 {
        self.event_id
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for &counter_id in &self.counter_ids {
            emit(counter_id);
        }
    }
}

#[derive(Clone, Default)]
struct Counter {
    counter_id: u128,
    time: Time,
    value: i64,
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

impl ApplyEvents<CounterChanged> for Counter {
    fn create(snapshot_id: u128, _first_event: &CounterChanged) -> Self {
        Self { counter_id: snapshot_id, ..Self::default() }
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, CounterChanged>) {
        for event in batch.events {
            self.value += event.delta;
        }
    }
}

fn main() {
    let config = ConTimeConfig {
        router_count: 1,
        worker_count: 4,
        placement: Placement::default(),
        pruning_interval: Duration::from_millis(100),
        history_retention: Time(1_000),
        checkpoints: CheckpointConfig { interval: 32 },
    };
    let history = ConTime::<CounterChanged, Counter, ()>::start(config, ()).expect("start ConTime");

    let start = Instant::now();
    let duration = Duration::from_secs(5);
    thread::scope(|scope| {
        scope.spawn(|| {
            let mut event_id = 0;
            loop {
                thread::sleep(Duration::from_millis(fastrand::u64(40..=180)));
                if start.elapsed() >= duration {
                    break;
                }
                event_id += 1;
                let counter_ids = match event_id % 3 {
                    0 => vec![1, 2],
                    1 => vec![1],
                    _ => vec![2],
                };
                let event =
                    CounterChanged { event_id, counter_ids, time: Time(start.elapsed().as_millis() as u64), delta: fastrand::i64(1..=5) };
                history.apply([event]).expect("enqueue counter change");
            }
        });

        let frame_interval = Duration::from_millis(50);
        let mut next_frame = start;
        while start.elapsed() < duration {
            let time = Time(start.elapsed().as_millis() as u64);
            history.advance_to(time).expect("advance processing target");
            let snapshots = history.query_at(time, [1, 2]).expect("query current counters");
            let values = [1, 2].map(|id| snapshots.iter().find(|snapshot| snapshot.counter_id == id).map_or(0, |snapshot| snapshot.value));
            println!("t={}ms counter 1={} counter 2={}", time.0, values[0], values[1]);
            for rejection in history.errors().try_iter() {
                eprintln!("rejected input: {rejection:?}");
            }
            next_frame += frame_interval;
            thread::sleep(next_frame.saturating_duration_since(Instant::now()));
        }
    });

    let report = history.shutdown();
    assert!(report.routers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
    assert!(report.workers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
}
