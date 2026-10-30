use std::thread;
use std::time::{Duration, Instant};

use contime::checkpoints::{CheckpointConfig, Timestamp};
use contime::{AdvanceTime, ConTime, ConTimeConfig, ContimeEvent, ContimeModule, ContimeSnapshot, Placement};

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

#[derive(ContimeEvent)]
#[contime_event(id = |event| event.event_id)]
struct CounterChanged {
    event_id: u128,
    counter_ids: Vec<u128>,
    time: Time,
    delta: i64,
}

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |counter_id| *counter_id, events = [CounterChanged => each(self.counter_ids, |id| [*id])], apply = {
    for event in batch.events {
        let CounterEvent::CounterChanged(change) = event;
        self.value += change.delta;
    }
})]
struct Counter {
    counter_id: u128,
    time: Time,
    value: i64,
}

#[derive(ContimeModule)]
#[contime_module(time = Time, events = [CounterChanged], snapshots = [Counter])]
struct Counters;

fn main() {
    let config = ConTimeConfig {
        router_count: 1,
        worker_count: 4,
        placement: Placement::default(),
        pruning_interval: Duration::from_millis(100),
        history_retention: Time(1_000),
        checkpoints: CheckpointConfig { interval: 32 },
    };
    let history = ConTime::<Counters>::start(config).expect("start ConTime");

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
                history.apply([event.into()]).expect("enqueue counter change");
            }
        });

        let frame_interval = Duration::from_millis(50);
        let mut next_frame = start;
        while start.elapsed() < duration {
            let time = Time(start.elapsed().as_millis() as u64);
            history.advance_to(time).expect("advance processing target");
            let snapshots = history.query_at(time, [Counter::snapshot_id(1), Counter::snapshot_id(2)]).expect("query current counters");
            let values = [1, 2].map(|id| {
                snapshots
                    .iter()
                    .find_map(|snapshot| {
                        let CountersSnapshotLanes::Counter(counter) = &**snapshot;
                        (counter.counter_id == id).then_some(counter.value)
                    })
                    .unwrap_or(0)
            });
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
