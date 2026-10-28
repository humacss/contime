//! Measures native submit-to-application and query response latency under load.
use contime::checkpoints::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, CheckpointConfig, Event, EventBatch, Snapshot, Timestamp};
use contime::{ConTime, ConTimeConfig, Input};
use crossbeam_channel::{select, tick, unbounded, Sender};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct Time(i64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime::AdvanceTime for Time {
    fn saturating_sub(&self, t: &Self) -> Self {
        Self(self.0.saturating_sub(t.0))
    }
}
struct Record {
    id: u128,
    history: u128,
    time: Time,
    sent: Instant,
}
impl Event for Record {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl Input for Record {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.history);
    }
}
#[derive(Clone)]
struct State {
    id: u128,
    time: Time,
    count: usize,
    hash: u64,
    sent: Instant,
}
fn hash(previous: u64, id: u128) -> u64 {
    previous.wrapping_add(id as u64).rotate_left(13).wrapping_mul(0x9e3779b185ebca87)
}
impl Snapshot for State {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, t: Time) {
        self.time = t;
    }
}
impl ApplyEvents<Record> for State {
    fn create(id: u128, first: &Record) -> Self {
        Self { id, time: Time(0), count: 0, hash: 0, sent: first.sent }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Record>) {
        for e in batch.events {
            self.hash = hash(self.hash, e.id);
            self.count += 1;
            self.sent = e.sent;
        }
    }
}
#[derive(Clone)]
struct Observe {
    target: Time,
    histories: usize,
    finished: usize,
    samples: Vec<u64>,
    done: Sender<Vec<u64>>,
}
impl ApplyWrapper<State, Record> for Observe {
    fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, Time, Record>, inner: &mut ApplyInner<'_, State>) {
        let last = batch.time == self.target;
        inner.apply_event_batch(batch);
        self.samples.push(inner.snapshot().sent.elapsed().as_nanos() as u64);
        if last {
            self.finished += 1;
            if self.finished == self.histories {
                self.done.send(std::mem::take(&mut self.samples)).unwrap();
            }
        }
    }
}
fn percentile(values: &mut [u64], percentile: usize) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_unstable();
    values[((values.len() - 1) * percentile).div_ceil(100)] as f64 / 1000.
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let number = |i, default| args.get(i).map_or(default, |s: &String| s.parse::<usize>().unwrap());
    let total = number(1, 262144);
    let workers = number(2, 4);
    let submission = number(3, 1);
    let arrivals = args.get(4).map_or("saturated", String::as_str);
    let query_us = number(5, 2000);
    assert!(total > 0 && total % 64 == 0 && workers > 0 && 64 % workers == 0 && submission > 0 && query_us > 0);
    assert!(["sparse", "burst", "saturated"].contains(&arrivals));
    let (done, completed) = unbounded();
    let target = Time((total / 64) as i64);
    let engine = ConTime::start(
        ConTimeConfig {
            router_count: 1,
            worker_count: workers,
            placement: Default::default(),
            pruning_interval: Duration::from_secs(3600),
            history_retention: Time(i64::MAX),
            checkpoints: CheckpointConfig { interval: 100 },
        },
        Observe { target, histories: 64 / workers, finished: 0, samples: Vec::new(), done },
    )
    .unwrap();
    engine.advance_to(target).unwrap();
    engine.wait_until_idle(Duration::from_secs(30)).unwrap();
    let mut samples = Vec::with_capacity(total);
    let mut queries = Vec::new();
    let started = Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let origin = Instant::now();
            for (range, offset) in submissions(total, submission, arrivals) {
                if let Some(wait) = (origin + offset).checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
                let sent = Instant::now();
                engine
                    .apply(range.map(|i| Record { id: i as u128 + 1, history: (i % 64) as u128, time: Time((i / 64 + 1) as i64), sent }))
                    .unwrap();
            }
        });
        let ticker = tick(Duration::from_micros(query_us as u64));
        let mut finished = 0;
        while finished < workers {
            select! {
                recv(completed) -> result => { samples.extend(result.unwrap());finished+=1; }
                recv(ticker) -> _ => {
                    let now=Instant::now();let result=engine.query_at(target,0..64).unwrap();
                    queries.push(now.elapsed().as_nanos() as u64);
                    assert!(result.iter().all(|s|s.count<=total/64));
                }
            }
        }
    });
    let elapsed = started.elapsed().as_secs_f64();
    assert_eq!(samples.len(), total);
    let mut expected = vec![0; 64];
    for i in 0..total {
        expected[i % 64] = hash(expected[i % 64], i as u128 + 1);
    }
    let actual = engine.query_at(target, 0..64).unwrap();
    assert_eq!(actual.len(), 64);
    for state in actual {
        assert_eq!(state.count, total / 64);
        assert_eq!(state.hash, expected[state.id as usize]);
    }
    assert_eq!(engine.errors().try_iter().count(), 0);
    let report = engine.shutdown();
    assert!(report.workers.iter().all(|r| *r == contime::contime_runtime::ThreadOutcome::Completed));
    let app = [50, 95, 99].map(|p| percentile(&mut samples, p));
    let query = [50, 95, 99].map(|p| percentile(&mut queries, p));
    println!("workers,total,submission,arrivals,seconds,events_per_sec,app_p50_us,app_p95_us,app_p99_us,query_count,query_p50_us,query_p95_us,query_p99_us");
    println!(
        "{workers},{total},{submission},{arrivals},{elapsed:.6},{:.0},{:.3},{:.3},{:.3},{},{:.3},{:.3},{:.3}",
        total as f64 / elapsed,
        app[0],
        app[1],
        app[2],
        queries.len(),
        query[0],
        query[1],
        query[2]
    );
}

// Arrival scheduling is separate from wall-clock sleeping so pacing can be tested.
fn submissions(total: usize, size: usize, arrivals: &str) -> impl Iterator<Item = (std::ops::Range<usize>, Duration)> + '_ {
    assert!(size > 0, "submission size must be positive");
    let (group, interval) = match arrivals {
        "saturated" => (usize::MAX, 0),
        "burst" => (1024, 5),
        "sparse" => (1, 1),
        _ => panic!("unknown arrival pattern"),
    };
    let mut start = 0;
    std::iter::from_fn(move || {
        if start == total {
            return None;
        }
        let remaining_in_group = group - start % group;
        let end = start + size.min(remaining_in_group).min(total - start);
        let due = Duration::from_millis((start / group) as u64 * interval);
        let range = start..end;
        start = end;
        Some((range, due))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_stream_has_no_submissions() {
        assert_eq!(submissions(0, 1, "burst").count(), 0);
    }
    #[test]
    #[should_panic(expected = "submission size")]
    fn zero_submission_is_rejected() {
        let _ = submissions(10, 0, "burst");
    }
    #[test]
    #[should_panic(expected = "unknown arrival")]
    fn unknown_arrival_is_rejected() {
        let _ = submissions(10, 1, "other");
    }
    #[test]
    fn pacing_is_independent_of_submission_size() {
        for size in [1, 127, 1000, 1024, 4096] {
            for (pattern, group, millis) in [("burst", 1024, 5), ("sparse", 1, 1), ("saturated", usize::MAX, 0)] {
                let mut seen = 0;
                for (range, due) in submissions(3073, size, pattern) {
                    assert_eq!(range.start, seen);
                    assert!(range.len() <= size);
                    for event in range.clone() {
                        assert_eq!(due, Duration::from_millis((event / group * millis) as u64), "{pattern}, size {size}, event {event}");
                    }
                    seen = range.end;
                }
                assert_eq!(seen, 3073);
            }
        }
    }
}
