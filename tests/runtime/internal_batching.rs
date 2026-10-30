//! Public requests retain their identity across native transport packet boundaries.
use contime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime::{ConTimeConfig, Input, RawConTime};
use crossbeam_channel::{unbounded, RecvTimeoutError};
use std::time::Duration;

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
impl contime::AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool {
        true
    }
    fn saturating_sub(&self, t: &Self) -> Self {
        Self(self.0.saturating_sub(t.0))
    }
}
struct Record {
    id: u128,
    history: u128,
    time: Time,
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
#[derive(Clone, Default)]
struct State {
    id: u128,
    time: Time,
    count: u64,
    hash: u64,
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
fn hash(previous: u64, id: u128) -> u64 {
    previous.wrapping_add(id as u64).rotate_left(13).wrapping_mul(0x9e3779b185ebca87)
}
impl ApplyEvents<Record> for State {
    fn create(id: u128, _: &Record) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Record>) {
        for event in batch.events {
            self.hash = hash(self.hash, event.id);
            self.count += 1;
        }
    }
}
#[test]
fn mixed_submission_sizes_and_distinct_responses_preserve_canonical_state() {
    const EVENTS: usize = 8192;
    const HISTORIES: usize = 16;
    for (routers, workers) in [(1, 1), (1, 4), (3, 4)] {
        let engine = RawConTime::<Record, State, ()>::start(
            ConTimeConfig {
                router_count: routers,
                worker_count: workers,
                placement: Default::default(),
                pruning_interval: Duration::from_millis(10),
                history_retention: Time(u64::MAX),
                checkpoints: CheckpointConfig { interval: 100 },
            },
            (),
        )
        .unwrap();
        let target = Time((EVENTS / HISTORIES) as u64);
        engine.advance_to(target).unwrap();
        let mut responses = Vec::new();
        let mut next = 0;
        for size in [1, 1000, 17].into_iter().cycle() {
            if next == EVENTS {
                break;
            }
            let end = (next + size).min(EVENTS);
            let (response, completed) = unbounded();
            engine
                .send(
                    (next..end).map(|i| Record {
                        id: i as u128 + 1,
                        history: (i % HISTORIES) as u128,
                        time: Time((i / HISTORIES + 1) as u64),
                    }),
                    response,
                )
                .unwrap();
            responses.push(completed);
            next = end;
        }
        for completed in responses {
            assert_eq!(completed.recv_timeout(Duration::from_secs(5)), Err(RecvTimeoutError::Disconnected));
        }
        engine.wait_until_idle(Duration::from_secs(5)).unwrap();
        let states = engine.query_at(target, 0..HISTORIES as u128).unwrap();
        assert_eq!(states.len(), HISTORIES);
        let mut expected = [0; HISTORIES];
        for i in 0..EVENTS {
            expected[i % HISTORIES] = hash(expected[i % HISTORIES], i as u128 + 1);
        }
        for state in states {
            assert_eq!(state.count, (EVENTS / HISTORIES) as u64);
            assert_eq!(state.hash, expected[state.id as usize]);
        }
        let report = engine.shutdown();
        assert!(report.workers.iter().all(|r| *r == contime::ThreadOutcome::Completed));
        assert!(report.routers.iter().all(|r| *r == contime::ThreadOutcome::Completed));
    }
}
