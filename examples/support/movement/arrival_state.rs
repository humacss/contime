//! Comparison state: change direction at actual handoff time, without replay.

use crate::movement_example::model::{Character, Motion, MovementInput, Time, SECOND};
use crate::support::movement_report::{Client, Point};
use std::collections::VecDeque;

struct Segment {
    state: Character,
    until: u64,
}
pub struct ArrivalState {
    states: Vec<Character>,
    segments: VecDeque<Segment>,
}
impl ArrivalState {
    pub fn new(clients: usize, motion: Motion) -> Self {
        Self { states: (1..=clients).map(|id| Character::spawn(id as u128, motion)).collect(), segments: VecDeque::new() }
    }
    pub fn deliver(&mut self, input: &MovementInput, now: u64) {
        let state = &mut self.states[input.client_id as usize - 1];
        self.segments.push_back(Segment { state: state.clone(), until: now });
        let mut arrived = input.clone();
        // Preserve handoff order, including several deliveries in one host turn.
        // Original action time is deliberately ignored by this comparison.
        arrived.time = Time(now);
        state.apply_input(&arrived);
        while self.segments.len() > 4096 {
            self.segments.pop_front();
        }
    }
    pub fn expire(&mut self, now: u64, trail_ms: u64) {
        let oldest = now.saturating_sub(trail_ms.saturating_mul(1_000_000));
        self.segments.retain(|segment| segment.until >= oldest);
    }
    // Later deliveries must not change earlier arrival-time movement.
    fn at(&self, client: u128, now: u64) -> Option<&Character> {
        let latest = self.states.get(client.checked_sub(1)? as usize)?;
        if latest.integrated_at.0 <= now {
            Some(latest)
        } else {
            self.segments
                .iter()
                .rev()
                .find(|segment| segment.state.client_id == client && segment.state.integrated_at.0 <= now && now < segment.until)
                .map(|segment| &segment.state)
        }
    }
    pub fn position(&self, client: u128, now: u64) -> Option<[i128; 2]> {
        self.at(client, now).map(|state| state.position_at(Time(now)))
    }
    pub fn view(&self, now: u64, trail_ms: u64) -> (Vec<Client>, Vec<Point>) {
        let world = |p: [i128; 2]| p.map(|v| v as f64 / SECOND as f64);
        let duration = trail_ms.saturating_mul(1_000_000).max(1);
        let oldest = now.saturating_sub(duration);
        let clients = self
            .states
            .iter()
            .filter_map(|latest| {
                let state = self.at(latest.client_id, now)?;
                Some(Client {
                    id: state.client_id as usize,
                    position: world(state.position_at(Time(now))),
                    direction: state.direction.velocity(1).map(|v| v as f64),
                    corrected: false,
                })
            })
            .collect();
        let trails = self
            .segments
            .iter()
            .map(|s| (&s.state, s.until.min(now)))
            .chain(self.states.iter().map(|state| (state, now)))
            .filter(|(state, end)| *end >= oldest && *end > state.integrated_at.0 && state.integrated_at.0 <= now)
            .map(|(state, end)| Point {
                previous: Some(world(state.position_at(Time(state.integrated_at.0.max(oldest))))),
                client: state.client_id as usize,
                time_ns: end,
                position: world(state.position_at(Time(end))),
                corrected: false,
                opacity: 0.18 + 0.82 * (1.0 - now.saturating_sub(end) as f64 / duration as f64).clamp(0.0, 1.0),
                current: true,
            })
            .collect();
        (clients, trails)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement_example::model::{Direction, Motion, MovementInput, Time, SECOND};

    fn input(id: u128, seconds: u64, direction: Direction) -> MovementInput {
        MovementInput { id, client_id: 1, time: Time(seconds * SECOND as u64), direction, motion: Motion::default() }
    }

    #[test]
    fn a_late_turn_changes_the_future_without_repairing_the_past() {
        let mut state = ArrivalState::new(1, Motion::default());
        state.deliver(&input(1, 0, Direction::East), 0);
        state.deliver(&input(2, 1, Direction::North), 2 * SECOND as u64);
        let (clients, _) = state.view(3 * SECOND as u64, 10_000);
        assert_eq!(clients[0].position, [12.0, 6.0]);
    }

    #[test]
    fn delivery_order_wins_even_when_action_timestamps_go_backwards() {
        let mut state = ArrivalState::new(1, Motion::default());
        state.deliver(&input(2, 2, Direction::East), 3 * SECOND as u64);
        state.deliver(&input(1, 1, Direction::North), 3 * SECOND as u64);
        let (clients, _) = state.view(4 * SECOND as u64, 10_000);
        assert_eq!(clients[0].position, [0.0, 6.0]);
    }

    #[test]
    fn zero_delay_square_closes_and_old_segments_are_bounded() {
        let mut state = ArrivalState::new(1, Motion::default());
        for (second, direction) in [Direction::East, Direction::South, Direction::West, Direction::North].into_iter().enumerate() {
            state.deliver(&input(second as u128 + 1, second as u64, direction), second as u64 * SECOND as u64);
        }
        assert_eq!(state.view(4 * SECOND as u64, 10_000).0[0].position, [0.0, 0.0]);
        for i in 4..10_000 {
            state.deliver(&input(i, i as u64, Direction::East), i as u64 * SECOND as u64);
        }
        state.expire(10_000 * SECOND as u64, 1000);
        assert!(state.segments.len() <= 2);
        assert!(state.view(10_000 * SECOND as u64, 1000).1.iter().all(|p| p.time_ns >= 9_999 * SECOND as u64));
    }
    #[test]
    fn later_deliveries_never_change_a_previous_display_time() {
        let mut state = ArrivalState::new(1, Motion::default());
        state.deliver(&input(1, 0, Direction::East), 0);
        let before = state.view(SECOND as u64, 10_000);
        state.deliver(&input(2, 1, Direction::North), 2 * SECOND as u64);
        let after = state.view(SECOND as u64, 10_000);
        assert_eq!(before.0[0].position, after.0[0].position);
        assert_eq!(before.0[0].direction, after.0[0].direction);
        assert!(after.1.iter().all(|p| p.time_ns <= SECOND as u64));
    }
    #[test]
    fn delivered_marker_keeps_its_actual_arrival_location_after_later_turns() {
        let mut state = ArrivalState::new(1, Motion::default());
        state.deliver(&input(1, 0, Direction::East), 0);
        state.deliver(&input(2, 1, Direction::North), 2 * SECOND as u64);
        state.deliver(&input(3, 0, Direction::West), 3 * SECOND as u64);
        assert_eq!(state.position(1, 2 * SECOND as u64), Some([12 * SECOND, 0]));
        assert_eq!(state.position(1, 3 * SECOND as u64), Some([12 * SECOND, 6 * SECOND]));
    }
}
