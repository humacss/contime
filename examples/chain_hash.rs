//! Interactive verification of order-sensitive state across a running history.
//!
//! Run `cargo run --example chain_hash -- --events-per-second 100`.
//! See `examples/chain_hash/README.md` for metrics and verification semantics.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use ratatui::crossterm::event::{self, Event as TerminalEvent};

#[path = "chain_hash/delivery.rs"]
mod delivery;
#[path = "chain_hash/metrics.rs"]
mod metrics;
#[path = "chain_hash/ui.rs"]
mod ui;
use std::time::{Duration, Instant};

use contime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime::{AdvanceTime, ConTime, ConTimeConfig, Input, Placement};

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct Time(u64);

impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}

#[derive(Clone)]
struct CounterChanged {
    event_id: u128,
    counter_ids: Vec<u128>,
    time: Time,
    delta: i64,
}

// Fixed-width fields and an explicit target count make the encoding unambiguous.
// The full previous digest carries the history into each new chain link.
fn chain_hash(previous: &[u8; 32], event: &CounterChanged) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"contime-counter-chain-v1");
    hasher.update(previous);
    hasher.update(&event.event_id.to_le_bytes());
    hasher.update(&event.time.0.to_le_bytes());
    hasher.update(&event.delta.to_le_bytes());
    hasher.update(&(event.counter_ids.len() as u64).to_le_bytes());
    for id in &event.counter_ids {
        hasher.update(&id.to_le_bytes());
    }
    *hasher.finalize().as_bytes()
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
    event_count: u64,
    hash: [u8; 32],
}

impl Counter {
    fn apply_change(&mut self, event: &CounterChanged) {
        self.value += event.delta;
        self.event_count += 1;
        self.hash = chain_hash(&self.hash, event);
    }
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
            self.apply_change(event);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    counters: usize,
    workers: usize,
    events_per_second: usize,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, String> {
        let mut options = Self { counters: 4, workers: 4, events_per_second: 10 };
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            if flag == "--help" || flag == "-h" {
                return Ok(None);
            }
            let field = match flag.as_str() {
                "--counters" => &mut options.counters,
                "--workers" => &mut options.workers,
                "--events-per-second" => &mut options.events_per_second,
                _ => return Err(format!("unknown option: {flag}")),
            };
            let value = args.next().ok_or_else(|| format!("{flag} requires a positive integer"))?;
            *field = value.parse().ok().filter(|value| *value > 0).ok_or_else(|| format!("{flag} requires a positive integer"))?;
        }
        if options.events_per_second > 1_000_000_000 {
            return Err("--events-per-second must not exceed 1000000000".to_owned());
        }
        Ok(Some(options))
    }
}

enum Notice {
    Produced(metrics::ProducedEvent),
    Delivered(delivery::Delivery),
    Failed(std::io::Error),
}

// Stop the producer even if drawing fails or the display loop unwinds.
struct StopProducer(Arc<AtomicBool>);

impl Drop for StopProducer {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

fn main() -> std::io::Result<()> {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("Usage: chain_hash [--counters N] [--workers N] [--events-per-second N]");
            println!("Defaults: 4 counters, 4 workers, 10 events per second. Live dashboard at 20 Hz. Quit with q, Esc, or Ctrl-C.");
            return Ok(());
        }
        Err(error) => {
            eprintln!("{error}\nUsage: chain_hash [--counters N] [--workers N] [--events-per-second N]");
            std::process::exit(2);
        }
    };
    let running = Arc::new(AtomicBool::new(true));
    let signal = Arc::clone(&running);
    ctrlc::set_handler(move || signal.store(false, Ordering::Relaxed)).expect("install Ctrl-C handler");
    let config = ConTimeConfig {
        router_count: 1,
        worker_count: options.workers,
        placement: Placement::default(),
        pruning_interval: Duration::from_millis(100),
        history_retention: Time(1_000_000_000),
        checkpoints: CheckpointConfig { interval: 32 },
    };
    let history = ConTime::<CounterChanged, Counter, ()>::start(config, ()).expect("start ConTime");
    let counter_ids = (1..=options.counters).map(|id| id as u128).collect::<Vec<_>>();
    let mut terminal = ratatui::init();
    let mut dashboard = ui::Dashboard::new(options.counters, options.workers, options.events_per_second);

    let start = Instant::now();
    let (notices, updates) = crossbeam_channel::unbounded();
    let mut tracker = metrics::Tracker::new(options.counters);
    let mut progress = delivery::Progress::default();

    let result: std::io::Result<()> = thread::scope(|scope| {
        let _stop_producer = StopProducer(Arc::clone(&running));
        scope.spawn(|| {
            let result = (|| -> std::io::Result<()> {
                let mut event_id = 0;
                let mut next_event = Instant::now();
                let mut last_time = Time::default();
                let mut group = Vec::new();
                let mut group_started = Instant::now();
                let mut primary = counter_ids[0];
                while running.load(Ordering::Relaxed) {
                    next_event += Duration::from_secs_f64((0.5 + fastrand::f64()) / options.events_per_second as f64);
                    while running.load(Ordering::Relaxed) && Instant::now() < next_event {
                        thread::sleep(next_event.saturating_duration_since(Instant::now()).min(Duration::from_millis(20)));
                    }
                    if !running.load(Ordering::Relaxed) {
                        break;
                    }
                    event_id += 1;
                    if group.is_empty() {
                        primary = counter_ids[fastrand::usize(0..counter_ids.len())];
                        group_started = Instant::now();
                    }
                    // Every group shares a primary history, so reversing it
                    // actually exercises order-sensitive state within a history.
                    let mut targets = vec![primary];
                    if event_id % 3 == 0 && counter_ids.len() > 1 {
                        targets.push(counter_ids[primary as usize % counter_ids.len()]);
                    }
                    // Unique timestamps prevent the next, incomplete group from
                    // sharing the timestamp of an acknowledged query frontier.
                    last_time = Time((start.elapsed().as_nanos() as u64).max(last_time.0 + 1));
                    let event = CounterChanged { event_id, counter_ids: targets, time: last_time, delta: fastrand::i64(1..=5) };
                    notices
                        .send(Notice::Produced(metrics::ProducedEvent { event: event.clone(), started: Instant::now() }))
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                    group.push(event);
                    // At low rates wait for at least two events; at high rates
                    // cap the group so delivery keeps progressing.
                    if group.len() >= 2 && (group_started.elapsed() >= Duration::from_millis(100) || group.len() >= 4096) {
                        let delivered = delivery::send_reversed(&history, std::mem::take(&mut group))?;
                        notices.send(Notice::Delivered(delivered)).map_err(|error| std::io::Error::other(error.to_string()))?;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                let _ = notices.send(Notice::Failed(error));
            }
        });

        (|| {
            let frame_interval = Duration::from_millis(50);
            while running.load(Ordering::Relaxed) {
                let deadline = Instant::now() + frame_interval;
                for notice in updates.try_iter().take(updates.len()) {
                    match notice {
                        Notice::Produced(submission) => tracker.record(submission),
                        Notice::Delivered(delivery) => progress.push(delivery),
                        Notice::Failed(error) => return Err(error),
                    }
                }
                progress.poll()?;
                // Queries only cover fully admitted canonical prefixes. While
                // delivery is incomplete, a valid partial set need not match
                // any prefix of the reference chain.
                let time = progress.through;
                dashboard.submitted = progress.submitted;
                dashboard.reordered = progress.reordered;
                dashboard.query_through = Duration::from_nanos(time.0);
                history.advance_to(time).expect("advance processing target");
                let query_started = Instant::now();
                let snapshots = history.query_at(time, counter_ids.iter().copied()).expect("query current counters");
                dashboard.query_time = query_started.elapsed();
                dashboard.values.fill(0);
                dashboard.verified_counts.fill(0);
                dashboard.hashes.fill([0; 32]);
                for snapshot in snapshots {
                    if let Some(value) = dashboard.values.get_mut((snapshot.counter_id - 1) as usize) {
                        *value = snapshot.value;
                        dashboard.verified_counts[(snapshot.counter_id - 1) as usize] = snapshot.event_count;
                        dashboard.hashes[(snapshot.counter_id - 1) as usize] = snapshot.hash;
                    }
                }
                let observed_at = Instant::now();
                tracker.observe(&dashboard.verified_counts, &dashboard.hashes, &dashboard.values, observed_at)?;
                terminal.draw(|frame| dashboard.draw(frame, start.elapsed(), tracker.metrics()))?;
                while running.load(Ordering::Relaxed) && Instant::now() < deadline {
                    if event::poll(deadline.saturating_duration_since(Instant::now()))? {
                        if let TerminalEvent::Key(key) = event::read()? {
                            if dashboard.handle_key(key) {
                                running.store(false, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
            Ok(())
        })()
    });
    ratatui::restore();

    let report = history.shutdown();
    assert!(report.routers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
    assert!(report.workers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
    result?;
    println!("ConTime stopped.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Options;

    #[test]
    fn configurable_rate_accepts_positive_values_and_rejects_invalid_ones() {
        assert_eq!(Options::parse(["--events-per-second", "100"].map(str::to_owned)).unwrap().unwrap().events_per_second, 100);
        for rate in ["0", "-1", "many", "1000000001"] {
            assert!(Options::parse(["--events-per-second", rate].map(str::to_owned)).is_err());
        }
    }

    #[test]
    fn chained_hash_detects_reordering_and_continues_from_a_snapshot() {
        let first = super::CounterChanged { event_id: 1, counter_ids: vec![1, 2], time: super::Time(1), delta: 3 };
        let second = super::CounterChanged { event_id: 2, counter_ids: vec![1], time: super::Time(2), delta: 4 };
        let initial = [0; 32];
        let checkpoint = super::chain_hash(&initial, &first);
        let ordered = super::chain_hash(&checkpoint, &second);
        let reversed = super::chain_hash(&super::chain_hash(&initial, &second), &first);
        assert_ne!(ordered, reversed);
        let mut snapshot = super::Counter::default();
        snapshot.apply_change(&first);
        let mut restored = snapshot.clone();
        restored.apply_change(&second);
        assert_eq!(restored.hash, ordered);
        assert_eq!(restored.event_count, 2);
        assert_eq!(restored.value, 7);
        let changed = super::CounterChanged { delta: 5, ..second };
        assert_ne!(ordered, super::chain_hash(&checkpoint, &changed));
    }

    #[test]
    fn queries_replay_hashes_in_canonical_order_across_shared_histories() {
        use super::*;
        let history = ConTime::<CounterChanged, Counter, ()>::start(
            ConTimeConfig {
                router_count: 1,
                worker_count: 2,
                placement: Placement::default(),
                pruning_interval: Duration::from_millis(100),
                history_retention: Time(1_000),
                checkpoints: CheckpointConfig { interval: 1 },
            },
            (),
        )
        .unwrap();
        let first = CounterChanged { event_id: 1, counter_ids: vec![1, 2], time: Time(1), delta: 2 };
        let second = CounterChanged { event_id: 2, counter_ids: vec![1], time: Time(2), delta: 3 };
        let third = CounterChanged { event_id: 3, counter_ids: vec![1, 2], time: Time(2), delta: 5 };
        let expected_one = chain_hash(&chain_hash(&chain_hash(&[0; 32], &first), &second), &third);
        let expected_two = chain_hash(&chain_hash(&[0; 32], &first), &third);
        // Insertion acknowledgements are only used here to make the test deterministic.
        let (responses, inserted) = crossbeam_channel::unbounded();
        history.send([third, first, second], responses).unwrap();
        assert!(inserted.iter().collect::<Vec<_>>().is_empty());
        history.advance_to(Time(3)).unwrap();
        for _ in 0..2 {
            let snapshots = history.query_at(Time(3), [1, 2]).unwrap();
            assert_eq!(snapshots.len(), 2);
            for snapshot in snapshots {
                let (hash, count, value) = if snapshot.counter_id == 1 { (expected_one, 3, 10) } else { (expected_two, 2, 7) };
                assert_eq!(snapshot.hash, hash);
                assert_eq!(snapshot.event_count, count);
                assert_eq!(snapshot.value, value);
            }
        }
        assert!(history.errors().is_empty());
        let report = history.shutdown();
        assert!(report.workers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
    }

    #[test]
    fn late_arrival_repairs_replay_and_hash_survives_pruning() {
        use super::*;
        let history = ConTime::<CounterChanged, Counter, ()>::start(
            ConTimeConfig {
                router_count: 1,
                worker_count: 2,
                placement: Placement::default(),
                pruning_interval: Duration::from_millis(10),
                history_retention: Time(1_000),
                checkpoints: CheckpointConfig { interval: 1 },
            },
            (),
        )
        .unwrap();
        let first = CounterChanged { event_id: 1, counter_ids: vec![1], time: Time(10), delta: 2 };
        let second = CounterChanged { event_id: 2, counter_ids: vec![1], time: Time(20), delta: 3 };
        let expected = chain_hash(&chain_hash(&[0; 32], &first), &second);
        let now = Instant::now();
        let mut tracker = metrics::Tracker::new(1);
        tracker.record(metrics::ProducedEvent { event: first.clone(), started: now });
        tracker.record(metrics::ProducedEvent { event: second.clone(), started: now });
        let insert = |event| {
            let (responses, inserted) = crossbeam_channel::unbounded();
            history.send([event], responses).unwrap();
            assert!(inserted.iter().collect::<Vec<_>>().is_empty());
        };
        insert(second);
        history.advance_to(Time(20)).unwrap();
        history.wait_until_idle(Duration::from_secs(2)).unwrap();
        let partial = history.query_at(Time(20), [1]).unwrap().pop().unwrap();
        // The partial delivery is valid ConTime state, but is not a prefix of
        // the generated reference. The live demo gates queries to avoid this.
        assert!(tracker.observe(&[partial.event_count], &[partial.hash], &[partial.value], now).is_err());
        insert(first);
        history.wait_until_idle(Duration::from_secs(2)).unwrap();
        let repaired = history.query_at(Time(20), [1]).unwrap().pop().unwrap();
        assert_eq!(repaired.hash, expected);
        tracker.observe(&[repaired.event_count], &[repaired.hash], &[repaired.value], now).unwrap();
        let pruned = history.subscribe_pruned_horizon().unwrap();
        history.advance_to(Time(1_020)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while pruned.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap() < Time(20) {}
        let retained = history.query_at(Time(1_020), [1]).unwrap().pop().unwrap();
        assert_eq!(retained.hash, expected);
        assert_eq!(retained.event_count, 2);
        tracker.observe(&[retained.event_count], &[retained.hash], &[retained.value], now).unwrap();
        assert_eq!(tracker.metrics().verified_events, 2);
        let report = history.shutdown();
        assert!(report.workers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
    }

    #[test]
    fn defaults_and_custom_topology() {
        assert_eq!(Options::parse([]).unwrap(), Some(Options { counters: 4, workers: 4, events_per_second: 10 }));
        let args = ["--counters", "7", "--workers", "2"].map(str::to_owned);
        assert_eq!(Options::parse(args).unwrap(), Some(Options { counters: 7, workers: 2, events_per_second: 10 }));
    }

    #[test]
    fn invalid_options_are_reported_before_starting_threads() {
        for args in [
            vec!["--counters", "0"],
            vec!["--workers", "0"],
            vec!["--counters", "-1"],
            vec!["--workers", "many"],
            vec!["--counters"],
            vec!["--unknown", "2"],
        ] {
            assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
        }
        assert_eq!(Options::parse(["--help".to_owned()]).unwrap(), None);
    }
}
