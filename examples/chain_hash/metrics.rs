use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::{chain_hash, CounterChanged};

pub struct ProducedEvent {
    pub event: CounterChanged,
    pub started: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderingCheck {
    NoEvents,
    Matches,
}

pub struct CounterCheck {
    pub produced_count: u64,
    pub verified_count: u64,
    pub expected_hash: Option<[u8; 32]>,
    pub produced_hash: [u8; 32],
    pub ordering: OrderingCheck,
}

pub struct Metrics {
    pub produced_values: Vec<i64>,
    pub checks: Vec<CounterCheck>,
    pub produced_events: usize,
    pub verified_events: usize,
    pub pending: usize,
    pub oldest_pending: Option<Duration>,
    pub last_delay: Option<Duration>,
    pub average_delay: Option<Duration>,
    pub max_delay: Option<Duration>,
}

struct Expected {
    count: u64,
    hash: [u8; 32],
    value: i64,
}

struct Pending {
    targets: Vec<(usize, u64)>,
    started: Instant,
}

pub struct Tracker {
    metrics: Metrics,
    expected: Vec<VecDeque<Expected>>,
    pending: Vec<Pending>,
    total_delay: Duration,
}

impl Tracker {
    pub fn new(counters: usize) -> Self {
        Self {
            metrics: Metrics {
                produced_values: vec![0; counters],
                checks: (0..counters)
                    .map(|_| CounterCheck {
                        produced_count: 0,
                        verified_count: 0,
                        expected_hash: Some([0; 32]),
                        produced_hash: [0; 32],
                        ordering: OrderingCheck::NoEvents,
                    })
                    .collect(),
                produced_events: 0,
                verified_events: 0,
                pending: 0,
                oldest_pending: None,
                last_delay: None,
                average_delay: None,
                max_delay: None,
            },
            expected: (0..counters).map(|_| VecDeque::from([Expected { count: 0, hash: [0; 32], value: 0 }])).collect(),
            pending: Vec::new(),
            total_delay: Duration::ZERO,
        }
    }

    pub fn record(&mut self, submission: ProducedEvent) {
        let mut targets = Vec::with_capacity(submission.event.counter_ids.len());
        for &id in &submission.event.counter_ids {
            let index = (id - 1) as usize;
            let previous = self.expected[index].back().expect("initial expected state is retained");
            let next = Expected {
                count: previous.count + 1,
                hash: chain_hash(&previous.hash, &submission.event),
                value: previous.value + submission.event.delta,
            };
            self.metrics.produced_values[index] = next.value;
            self.metrics.checks[index].produced_count = next.count;
            self.metrics.checks[index].produced_hash = next.hash;
            targets.push((index, next.count));
            self.expected[index].push_back(next);
        }
        self.metrics.produced_events += 1;
        self.pending.push(Pending { targets, started: submission.started });
    }

    pub fn observe(&mut self, counts: &[u64], hashes: &[[u8; 32]], values: &[i64], now: Instant) -> std::io::Result<()> {
        // Search by digest, rather than trusting the snapshot's claimed count.
        // Validate every result before discarding any reference history.
        let positions = self
            .expected
            .iter()
            .enumerate()
            .map(|(index, history)| {
                let position = history.iter().position(|state| state.hash == hashes[index]).ok_or_else(|| {
                    std::io::Error::other(format!(
                    "fatal hash verification failure for counter {} at event {}: queried {} is absent from the retained reference chain",
                    index + 1, counts[index], blake3::Hash::from_bytes(hashes[index])
                ))
                })?;
                let expected = &history[position];
                if expected.count != counts[index] || expected.value != values[index] {
                    return Err(std::io::Error::other(format!(
                        "fatal state verification failure for counter {}: digest matches but event count or total differs",
                        index + 1
                    )));
                }
                Ok(position)
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        for (index, position) in positions.into_iter().enumerate() {
            let check = &mut self.metrics.checks[index];
            check.verified_count = counts[index];
            check.expected_hash = Some(hashes[index]);
            check.ordering = if counts[index] == 0 { OrderingCheck::NoEvents } else { OrderingCheck::Matches };
            // Keep the match itself as the next baseline, plus newer digests.
            self.expected[index].drain(..position);
        }
        self.pending.retain(|submission| {
            if !submission
                .targets
                .iter()
                .all(|&(index, count)| counts[index] >= count && self.metrics.checks[index].ordering == OrderingCheck::Matches)
            {
                return true;
            }
            let delay = now.saturating_duration_since(submission.started);
            self.metrics.verified_events += 1;
            self.total_delay += delay;
            self.metrics.last_delay = Some(delay);
            self.metrics.max_delay = Some(self.metrics.max_delay.unwrap_or_default().max(delay));
            false
        });
        self.metrics.pending = self.pending.len();
        self.metrics.oldest_pending = self.pending.iter().map(|submission| now.saturating_duration_since(submission.started)).max();
        if self.metrics.verified_events > 0 {
            self.metrics.average_delay = Some(self.total_delay.div_f64(self.metrics.verified_events as f64));
        }
        Ok(())
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Counter, Time};

    fn event(event_id: u128, ids: &[u128]) -> CounterChanged {
        CounterChanged { event_id, counter_ids: ids.to_vec(), delta: 3, time: Time(event_id as u64) }
    }

    fn observe(tracker: &mut Tracker, counters: &[Counter], now: Instant) {
        tracker
            .observe(
                &counters.iter().map(|c| c.event_count).collect::<Vec<_>>(),
                &counters.iter().map(|c| c.hash).collect::<Vec<_>>(),
                &counters.iter().map(|c| c.value).collect::<Vec<_>>(),
                now,
            )
            .unwrap();
    }

    #[test]
    fn unknown_hash_is_a_fatal_error_and_preserves_the_reference_chain() {
        let mut tracker = Tracker::new(1);
        let start = Instant::now();
        tracker.record(ProducedEvent { event: event(1, &[1]), started: start });
        let result = tracker.observe(&[1], &[[42; 32]], &[3], start);
        assert!(result.is_err());
        assert_eq!(tracker.expected[0].len(), 2);
    }

    #[test]
    fn shared_event_waits_for_all_targets_and_is_measured_once() {
        let start = Instant::now();
        let mut tracker = Tracker::new(2);
        let event = event(1, &[1, 2]);
        tracker.record(ProducedEvent { event: event.clone(), started: start });
        let mut counters = [Counter::default(), Counter::default()];
        counters[0].apply_change(&event);
        observe(&mut tracker, &counters, start + Duration::from_millis(20));
        assert_eq!(tracker.metrics().produced_values, [3, 3]);
        assert_eq!(tracker.metrics().pending, 1);
        assert_eq!(tracker.metrics().verified_events, 0);
        assert_eq!(tracker.metrics().checks[0].ordering, OrderingCheck::Matches);
        assert_eq!(tracker.metrics().checks[1].ordering, OrderingCheck::NoEvents);
        counters[1].apply_change(&event);
        observe(&mut tracker, &counters, start + Duration::from_millis(50));
        observe(&mut tracker, &counters, start + Duration::from_millis(100));
        assert_eq!(tracker.metrics().verified_events, 1);
        assert_eq!(tracker.metrics().pending, 0);
        assert_eq!(tracker.metrics().last_delay, Some(Duration::from_millis(50)));
    }

    #[test]
    fn independent_histories_complete_out_of_submission_order() {
        let start = Instant::now();
        let mut tracker = Tracker::new(2);
        let first = event(1, &[1]);
        let second = event(2, &[2]);
        tracker.record(ProducedEvent { event: first.clone(), started: start });
        tracker.record(ProducedEvent { event: second.clone(), started: start });
        let mut counters = [Counter::default(), Counter::default()];
        counters[1].apply_change(&second);
        observe(&mut tracker, &counters, start + Duration::from_millis(20));
        assert_eq!(tracker.metrics().verified_events, 1);
        assert_eq!(tracker.metrics().oldest_pending, Some(Duration::from_millis(20)));
        counters[0].apply_change(&first);
        observe(&mut tracker, &counters, start + Duration::from_millis(40));
        assert_eq!(tracker.metrics().average_delay, Some(Duration::from_millis(30)));
        assert_eq!(tracker.metrics().max_delay, Some(Duration::from_millis(40)));
    }

    #[test]
    fn a_lagging_counter_matches_its_visible_prefix_not_the_latest_hash() {
        let start = Instant::now();
        let mut tracker = Tracker::new(1);
        let first = event(1, &[1]);
        let second = event(2, &[1]);
        tracker.record(ProducedEvent { event: first.clone(), started: start });
        tracker.record(ProducedEvent { event: second, started: start });
        let mut counter = Counter::default();
        counter.apply_change(&first);
        observe(&mut tracker, &[counter], start);
        let check = &tracker.metrics().checks[0];
        assert_eq!(check.produced_count, 2);
        assert_eq!(check.verified_count, 1);
        assert_eq!(check.ordering, OrderingCheck::Matches);
        assert_eq!(tracker.metrics().verified_events, 1);
        assert_eq!(tracker.metrics().pending, 1);
    }

    #[test]
    fn equal_totals_with_wrong_order_fail_without_discarding_reference_history() {
        let start = Instant::now();
        let mut tracker = Tracker::new(1);
        let first = event(1, &[1]);
        let second = event(2, &[1]);
        tracker.record(ProducedEvent { event: first.clone(), started: start });
        tracker.record(ProducedEvent { event: second.clone(), started: start });
        let mut wrong = Counter::default();
        wrong.apply_change(&second);
        wrong.apply_change(&first);
        assert_eq!(wrong.value, tracker.metrics().produced_values[0]);
        assert!(tracker.observe(&[wrong.event_count], &[wrong.hash], &[wrong.value], start).is_err());
        assert_eq!(tracker.expected[0].len(), 3);
        assert_eq!(tracker.metrics().verified_events, 0);
    }

    #[test]
    fn verifier_discards_old_expectations_but_preserves_the_chain() {
        let start = Instant::now();
        let mut tracker = Tracker::new(1);
        let mut counter = Counter::default();
        for id in 1..=200 {
            let event = event(id, &[1]);
            tracker.record(ProducedEvent { event: event.clone(), started: start });
            counter.apply_change(&event);
            observe(&mut tracker, std::slice::from_ref(&counter), start);
            assert_eq!(tracker.expected[0].len(), 1);
            assert_eq!(tracker.metrics().checks[0].ordering, OrderingCheck::Matches);
        }
    }

    #[test]
    fn a_matching_hash_with_corrupt_count_or_total_is_fatal() {
        let start = Instant::now();
        let mut tracker = Tracker::new(1);
        assert!(tracker.observe(&[1], &[[0; 32]], &[0], start).is_err());
        assert!(tracker.observe(&[0], &[[0; 32]], &[1], start).is_err());
    }
}
