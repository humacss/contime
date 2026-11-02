//! Synchronized bot inputs; client one is reserved for keyboard input.
use crate::movement_example::model::{Direction, Motion, MovementInput, Time};
use contime::contime_simulator::{EventStream, StreamConfig};

/// All bots turn on the same period boundaries; the player remains independent.
/// An absolute schedule avoids drifting or bunching inputs at host frame boundaries.
pub fn scheduled(clients: usize, hz: u64, index: u64) -> Option<(usize, Time)> {
    let bots = clients.checked_sub(1).filter(|&n| n > 0)? as u64;
    let period = 1_000_000_000u64.checked_div(hz)?;
    let client = index % bots + 2;
    let time = (index / bots).saturating_add(1).saturating_mul(period).saturating_add(1);
    Some((client as usize, Time(time)))
}

fn square_direction(client: usize, turn: u64) -> Direction {
    // Direction indices 1–4 are North, East, South, West; skip zero (Stop).
    let phase = (turn % 4 + client.saturating_sub(1) as u64 % 4) % 4;
    let direction = Direction::from_index(phase + 1);
    // Keep purple on the outer left and blue nearer the center; the two
    // right-side routes retain their existing alignment.
    match (client, direction) {
        (3 | 5, Direction::East) => Direction::West,
        (3 | 5, Direction::West) => Direction::East,
        _ => direction,
    }
}

pub fn event(client: usize, sequence: u64, time: Time, motion: Motion, turn: u64) -> MovementInput {
    EventStream::new(
        StreamConfig { event_count: 1, seed: sequence ^ 42, settings: () },
        move |_, _| MovementInput {
            id: u128::from(sequence),
            client_id: 0,
            time,
            // Equal-duration turns close a square; client phases keep paths distinct.
            direction: square_direction(client, turn),
            motion,
        },
        move |_, _, event: &mut MovementInput| {
            event.client_id = client as u128;
        },
    )
    .next()
    .expect("one configured input")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bots_share_source_ticks_and_repeat_once_per_second() {
        let slots: Vec<_> = (0..8).map(|i| scheduled(5, 1, i).unwrap()).collect();
        assert_eq!(slots[..4], [(2, Time(1_000_000_001)), (3, Time(1_000_000_001)), (4, Time(1_000_000_001)), (5, Time(1_000_000_001))]);
        for i in 0..4 {
            assert_eq!(slots[i].0, slots[i + 4].0);
            assert_eq!(slots[i + 4].1 .0 - slots[i].1 .0, 1_000_000_000);
        }
        assert_eq!(scheduled(1, 1, 0), None);
        assert_eq!(scheduled(2, 2, 1), Some((2, Time(1_000_000_001))));
    }
    #[test]
    fn all_bot_counts_and_configured_frequencies_keep_aligned_action_times() {
        for clients in 2..=5 {
            for hz in [1, 2, 60] {
                for tick in 0..4 {
                    let events: Vec<_> =
                        (0..clients - 1).map(|bot| scheduled(clients, hz, (tick * (clients - 1) + bot) as u64).unwrap()).collect();
                    assert!(events.iter().all(|(_, time)| *time == events[0].1));
                    assert_eq!(events[0].1 .0, (tick as u64 + 1) * (1_000_000_000 / hz) + 1);
                }
            }
        }
        assert_eq!(scheduled(5, 0, 0), None);
    }
    #[test]
    fn bots_generate_all_four_movement_directions_and_never_stop() {
        let mut seen = [false; 4];
        for sequence in 1..=4096 {
            let input = event(2, sequence, Time(sequence), Motion::default(), sequence);
            let index = match input.direction {
                Direction::North => 0,
                Direction::East => 1,
                Direction::South => 2,
                Direction::West => 3,
                Direction::Stop => panic!("bots must not generate Stop"),
            };
            seen[index] = true;
        }
        assert!(seen.into_iter().all(|seen| seen));
        assert_eq!(Direction::Stop.velocity(12), [0, 0], "Stop remains supported for player input");
    }
    #[test]
    fn every_bot_repeats_a_closed_square_with_equal_sides() {
        use crate::movement_example::model::{Character, SECOND};
        for client in 2..=5 {
            let mut state = Character::spawn(client, Motion::default());
            let origin = state.position;
            for turn in 0..40 {
                let input = event(client as usize, turn + 1, Time(turn * SECOND as u64), Motion::default(), turn);
                state.apply_input(&input);
                if turn % 4 == 3 {
                    assert_eq!(state.position_at(Time((turn + 1) * SECOND as u64)), origin);
                }
            }
        }
    }
    #[test]
    fn generated_input_keeps_its_assigned_client_time_and_identity() {
        let input = event(3, 12, Time(400_000_001), Motion::default(), 0);
        assert_eq!((input.id, input.client_id, input.time), (12, 3, Time(400_000_001)));
    }
    #[test]
    fn showcase_scale_keeps_each_square_in_its_own_quadrant() {
        use crate::movement_example::model::{Character, SECOND};
        let motion = Motion { client_spacing: 16, movement_speed: 12 };
        for client in 2..=5 {
            let mut state = Character::spawn(client, motion);
            for turn in 0..20 {
                state.apply_input(&event(client as usize, turn + 1, Time(turn * SECOND as u64), motion, turn));
                let [x, y] = state.position_at(Time((turn + 1) * SECOND as u64));
                assert_eq!(x > 0, matches!(client, 2 | 4));
                assert_eq!(y > 0, matches!(client, 4 | 5));
                assert!(x.abs() >= 4 * SECOND && y.abs() >= 8 * SECOND);
                if client == 3 {
                    assert!(x >= -16 * SECOND, "blue square stays right of its origin");
                } else if client == 5 {
                    assert!(x <= -16 * SECOND, "purple square stays left of its origin");
                }
            }
        }
    }
}
