//! Simulated transport; it owns inputs until their scheduled delivery, not ConTime.
use crate::movement_example::model::{MovementInput, Time};
use crate::support::movement_report::{NetworkEvent, MAX_NETWORK_EVENTS};
use contime::contime_simulator::Random;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    num::NonZeroU64,
};
pub struct Delivery {
    recent: VecDeque<NetworkEvent>,
    waiting: BTreeMap<(u64, u128), MovementInput>,
    last: HashMap<u128, Time>,
    random: Random,
    pub late: u64,
}
impl Delivery {
    pub fn new() -> Self {
        Self { recent: VecDeque::new(), waiting: BTreeMap::new(), last: HashMap::new(), random: Random::new(42), late: 0 }
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
                let reordered = event.time < *previous;
                if reordered {
                    self.late += 1;
                }
                *previous = (*previous).max(event.time);
                self.recent.push_back(NetworkEvent {
                    id: event.id as u64,
                    client: event.client_id as usize,
                    direction: event.direction.velocity(1).map(|v| v as f64),
                    action_ns: event.time.0,
                    delivery_ns: key.0,
                    arrived_ns: Some(now),
                    reordered,
                });
                while self.recent.len() > MAX_NETWORK_EVENTS {
                    self.recent.pop_front();
                }
                event
            })
            .collect()
    }
    /// A bounded next-to-arrive preview plus recent arrivals, displayed in action order.
    pub fn timeline(&self, now: u64, past_ns: u64) -> Vec<NetworkEvent> {
        let mut rows: Vec<_> = self
            .waiting
            .iter()
            .take(4)
            .map(|(&(due, _), event)| NetworkEvent {
                id: event.id as u64,
                client: event.client_id as usize,
                direction: event.direction.velocity(1).map(|v| v as f64),
                action_ns: event.time.0,
                delivery_ns: due,
                arrived_ns: None,
                reordered: false,
            })
            .collect();
        rows.extend(
            self.recent
                .iter()
                .rev()
                .filter(|e| e.arrived_ns.is_some_and(|at| now.saturating_sub(at) <= past_ns))
                .take(MAX_NETWORK_EVENTS - rows.len())
                .cloned(),
        );
        rows.sort_by_key(|e| (e.action_ns, e.id));
        rows
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
    fn demonstration_delays_regularly_reorder_inputs_for_every_bot() {
        let mut queue = Delivery::new();
        let config = crate::support::movement_config::Config::parse(std::iter::empty()).unwrap().unwrap();
        let mut id = 0;
        for second in 0..80 {
            for client in 2..=5 {
                id += 1;
                let at = second * 1_000_000_000 + (client - 1) * 200_000_000;
                queue.schedule(
                    MovementInput { id, client_id: client.into(), time: Time(at), direction: Direction::East, motion: Motion::default() },
                    at,
                    config.average_latency_ms,
                    config.jitter_ms,
                );
            }
        }
        let mut latest = [Time(0); 6];
        let mut reordered = [0; 6];
        for input in queue.take_ready(100_000_000_000) {
            let client = input.client_id as usize;
            if input.time < latest[client] {
                reordered[client] += 1;
            }
            latest[client] = latest[client].max(input.time);
        }
        for count in &reordered[2..] {
            assert!(*count >= 2, "each bot must demonstrate repeated out-of-order delivery: {reordered:?}");
        }
    }
    #[test]
    fn timeline_preserves_action_due_and_actual_reordered_arrival_times() {
        let mut queue = Delivery::new();
        let input = |id, time| MovementInput { id, client_id: 1, time: Time(time), direction: Direction::East, motion: Motion::default() };
        queue.schedule(input(1, 1), 1, 100, 0);
        queue.schedule(input(2, 2), 2, 0, 0);
        let waiting = queue.timeline(2, 3_428_571_428);
        assert_eq!(waiting.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 2]);
        assert!(waiting.iter().all(|e| e.arrived_ns.is_none()));
        queue.take_ready(10);
        let recent = queue.timeline(10, 3_428_571_428);
        let newer = recent.iter().find(|e| e.id == 2).unwrap();
        assert_eq!((newer.action_ns, newer.delivery_ns, newer.arrived_ns), (2, 2, Some(10)));
        assert!(!newer.reordered);
        queue.take_ready(100_000_010);
        let recent = queue.timeline(100_000_010, 3_428_571_428);
        let older = recent.iter().find(|e| e.id == 1).unwrap();
        assert_eq!(older.delivery_ns, 100_000_001);
        assert_eq!(older.arrived_ns, Some(100_000_010));
        assert!(older.reordered);
        assert!(queue.timeline(4_000_000_000, 3_428_571_428).is_empty());
    }
    #[test]
    fn delivered_inputs_remain_until_the_entire_delay_line_leaves_the_view() {
        let mut queue = Delivery::new();
        let past = crate::support::movement_report::timeline_window(600, 600).0 as u64;
        for id in 1..=16 {
            let at = id * 200_000_000;
            queue.schedule(
                MovementInput {
                    id: id.into(),
                    client_id: (id % 4 + 2).into(),
                    time: Time(at),
                    direction: Direction::East,
                    motion: Motion::default(),
                },
                at,
                0,
                0,
            );
            queue.take_ready(at);
        }
        assert_eq!(queue.timeline(3_200_000_000, past).len(), 16, "the old eight-input cap must not truncate history");
        let delivered = 200_000_000;
        assert!(queue.timeline(delivered + past, past).iter().any(|e| e.id == 1));
        assert!(!queue.timeline(delivered + past + 1, past).iter().any(|e| e.id == 1));
    }
    #[test]
    fn timeline_stays_bounded_with_a_large_queue_and_delivery_burst() {
        let mut queue = Delivery::new();
        for id in 1..=200 {
            queue.schedule(
                MovementInput { id, client_id: 1, time: Time(id as u64), direction: Direction::Stop, motion: Motion::default() },
                id as u64,
                100,
                0,
            );
        }
        assert_eq!(queue.timeline(100, 3_428_571_428).len(), 4);
        queue.take_ready(2_000_000_000);
        assert_eq!(queue.recent.len(), MAX_NETWORK_EVENTS);
        let rows = queue.timeline(2_000_000_000, 3_428_571_428);
        assert_eq!(rows.len(), MAX_NETWORK_EVENTS);
        assert!(rows.windows(2).all(|pair| pair[0].action_ns < pair[1].action_ns));
    }
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
