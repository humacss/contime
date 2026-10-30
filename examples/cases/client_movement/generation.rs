//! Staggered bot inputs; client one is reserved for keyboard input.
use crate::movement_example::model::{Direction, Motion, MovementInput, Time};
use contime::contime_simulator::{EventStream, StreamConfig};

/// Divide each input period into client slots, reserving the first for the player.
/// An absolute schedule avoids drifting or bunching inputs at host frame boundaries.
pub fn scheduled(clients: usize, hz: u64, index: u64) -> Option<(usize, Time)> {
    let bots = clients.checked_sub(1).filter(|&n| n > 0)? as u64;
    let period = 1_000_000_000 / hz;
    let client = index % bots + 2;
    let time = (index / bots).saturating_mul(period).saturating_add((client - 1) * period / clients as u64).saturating_add(1);
    Some((client as usize, Time(time)))
}

pub fn event(client: usize, sequence: u64, time: Time, motion: Motion) -> MovementInput {
    EventStream::new(
        StreamConfig { event_count: 1, seed: sequence ^ 42, settings: () },
        move |_, random| MovementInput {
            id: u128::from(sequence),
            client_id: 0,
            time,
            // Indices 1–4 are movement; zero (Stop) is reserved for the player.
            direction: Direction::from_index(random.next_u64() % 4 + 1),
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
    fn bots_have_separate_slots_and_repeat_once_per_second() {
        let slots: Vec<_> = (0..8).map(|i| scheduled(5, 1, i).unwrap()).collect();
        assert_eq!(slots[..4], [(2, Time(200_000_001)), (3, Time(400_000_001)), (4, Time(600_000_001)), (5, Time(800_000_001))]);
        for i in 0..4 {
            assert_eq!(slots[i].0, slots[i + 4].0);
            assert_eq!(slots[i + 4].1 .0 - slots[i].1 .0, 1_000_000_000);
        }
        assert_eq!(scheduled(1, 1, 0), None);
        assert_eq!(scheduled(2, 2, 1), Some((2, Time(750_000_001))));
    }
    #[test]
    fn bots_generate_all_four_movement_directions_and_never_stop() {
        let mut seen = [false; 4];
        for sequence in 1..=4096 {
            let input = event(2, sequence, Time(sequence), Motion::default());
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
    fn generated_input_keeps_its_assigned_client_time_and_identity() {
        let input = event(3, 12, Time(400_000_001), Motion::default());
        assert_eq!((input.id, input.client_id, input.time), (12, 3, Time(400_000_001)));
    }
}
