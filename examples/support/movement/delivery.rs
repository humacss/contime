//! Simulated transport; it owns inputs until their scheduled delivery, not ConTime.
use crate::movement_example::model::{MovementInput, Time};
use contime_simulator::Random;
use std::{
    collections::{BTreeMap, HashMap},
    num::NonZeroU64,
};
pub struct Delivery {
    waiting: BTreeMap<(u64, u128), MovementInput>,
    last: HashMap<u128, Time>,
    random: Random,
    pub late: u64,
}
impl Delivery {
    pub fn new() -> Self {
        Self { waiting: BTreeMap::new(), last: HashMap::new(), random: Random::new(42), late: 0 }
    }
    pub fn schedule(&mut self, event: MovementInput, now: u64, average: u64, jitter: u64) {
        // Config validation bounds these values to the retained time range.
        // Cap symmetric jitter rather than clamping individual samples at zero,
        // which would raise the requested mean for low-latency configurations.
        let radius = average.min(jitter);
        let width = NonZeroU64::new(2 * radius + 1).expect("inclusive jitter range is nonzero");
        let delay = average - radius + self.random.below(width);
        self.waiting.insert((now.saturating_add(delay * 1_000_000), event.id), event);
    }
    pub fn take_ready(&mut self, now: u64) -> Vec<MovementInput> {
        let keys: Vec<_> = self.waiting.range(..=(now, u128::MAX)).take(1024).map(|(key, _)| *key).collect();
        keys.into_iter()
            .map(|key| {
                let event = self.waiting.remove(&key).expect("selected queued input");
                let previous = self.last.entry(event.client_id).or_insert(Time(0));
                if event.time < *previous {
                    self.late += 1;
                }
                *previous = (*previous).max(event.time);
                event
            })
            .collect()
    }
    pub fn len(&self) -> usize {
        self.waiting.len()
    }
    /// Keep the query clock current while preserving any undelivered input.
    pub fn query_target(&self, active: Time, retention_ms: u64) -> Time {
        self.oldest_time().map_or(active, |oldest| active.min(Time(oldest.0.saturating_add(retention_ms * 1_000_000).saturating_sub(1))))
    }
    fn oldest_time(&self) -> Option<Time> {
        self.waiting.values().map(|e| e.time).min()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement_example::model::{Direction, Motion};
    #[test]
    fn long_latency_does_not_unnecessarily_hold_back_the_query_clock() {
        let mut queue = Delivery::new();
        queue.schedule(
            MovementInput { id: 1, client_id: 1, time: Time(1), direction: Direction::East, motion: Motion::default() },
            1,
            30000,
            0,
        );
        assert_eq!(queue.query_target(Time(25_000_000_000), 32801), Time(25_000_000_000));
        let bounded = queue.query_target(Time(40_000_000_000), 32801);
        assert!(bounded.0.saturating_sub(32_801_000_000) < 1, "old input must stay inside retention");
    }
    #[test]
    fn sampled_delay_has_the_requested_average_even_when_jitter_exceeds_it() {
        for average in [0u64, 50, 1000, 30000] {
            let mut queue = Delivery::new();
            let count = 20000;
            for id in 1..=count {
                queue.schedule(
                    MovementInput { id, client_id: 1, time: Time(1), direction: Direction::East, motion: Motion::default() },
                    0,
                    average,
                    600,
                );
            }
            let radius = average.min(600);
            let mut sum = 0u128;
            for &(due, _) in queue.waiting.keys() {
                let delay = due / 1_000_000;
                assert!((average - radius..=average + radius).contains(&delay), "average {average}: delay {delay}");
                sum += u128::from(delay);
            }
            let observed = (sum / count) as u64;
            assert!(observed.abs_diff(average) <= 2, "requested {average} ms, observed {observed} ms");
        }
    }
    #[test]
    fn an_older_turn_arrives_after_a_newer_turn_without_changing_its_timestamp() {
        let mut queue = Delivery::new();
        queue.schedule(
            MovementInput { id: 1, client_id: 1, time: Time(1), direction: Direction::East, motion: Motion::default() },
            0,
            100,
            0,
        );
        queue.schedule(
            MovementInput { id: 2, client_id: 1, time: Time(2), direction: Direction::North, motion: Motion::default() },
            0,
            0,
            0,
        );
        assert_eq!(queue.take_ready(0)[0].id, 2);
        let late = queue.take_ready(100_000_000);
        assert_eq!(late[0].time, Time(1));
        assert_eq!(queue.late, 1);
        assert_eq!(queue.len(), 0);
    }
}
