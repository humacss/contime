//! Choreographed inputs for the README recording, not the interactive generator.
//! Repeat small loops away from the walls. Delays deliberately let a later turn
//! overtake an earlier one; positions and corrections still come from real queries.
use crate::movement_example::model::{Direction, Motion, MovementInput, Time};

pub fn input(index: u64, motion: Motion) -> (MovementInput, u64) {
    use Direction::{East, North, South, Stop, West};
    let client = index % 5 + 1;
    let second = index / 5;
    let phase = second as usize % 8;
    let direction = match client {
        // An eight-second square followed by a rest makes Stop easy to recognize.
        1 => [East, East, South, South, West, West, North, North, Stop, Stop, Stop, Stop][second as usize % 12],
        // First move into separate quadrants, then loop within each region.
        2 | 5 if second < 2 => North,
        3 | 4 if second < 2 => South,
        2 => [West, South, East, North][(second as usize - 2) % 4],
        3 => [East, North, West, South][(second as usize - 2) % 4],
        4 => [West, North, East, South][(second as usize - 2) % 4],
        _ => [East, South, West, North][(second as usize - 2) % 4],
    };
    let time = Time(second * 1_000_000_000 + (client - 1) * 200_000_000 + 1);
    let delay_ms = [1700, 150, 700, 1150, 1550, 200, 900, 450][(phase + client as usize * 2) % 8];
    (MovementInput { id: u128::from(index + 6), client_id: client.into(), time, direction, motion }, delay_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement_example::model::{Character, SECOND};
    #[test]
    fn choreography_is_repeatable_stays_inside_walls_and_keeps_bots_moving() {
        let motion = Motion { client_spacing: 18, movement_speed: 8 };
        let mut states: Vec<_> = (1..=5).map(|id| Character::spawn(id, motion)).collect();
        let mut stopped = false;
        for index in 0..600 {
            let (event, delay) = input(index, motion);
            let (again, again_delay) = input(index, motion);
            assert_eq!((event.id, event.time, event.direction, delay), (again.id, again.time, again.direction, again_delay));
            assert!(delay <= 2000);
            if event.client_id != 1 {
                assert_ne!(event.direction, Direction::Stop);
            }
            stopped |= event.direction == Direction::Stop;
            let state = &mut states[event.client_id as usize - 1];
            state.apply_input(&event);
            if event.time.0 >= 3_000_000_000 && event.client_id != 1 {
                let [x, y] = state.position;
                let (right, upper) = match event.client_id {
                    2 => (true, true),
                    3 => (false, false),
                    4 => (true, false),
                    _ => (false, true),
                };
                assert_eq!(x > 0, right);
                assert_eq!(y > 0, upper);
                assert!(x.abs() >= 10 * SECOND && y.abs() >= 8 * SECOND);
            }
            // Even extrapolation through the longest delay leaves a safety margin.
            let position = state.position_at(Time(event.time.0 + 1_700_000_000));
            assert!(position[0].abs() < 47 * SECOND && position[1].abs() < 27 * SECOND, "{index}: {position:?}");
        }
        assert!(stopped);
    }
    #[test]
    fn every_client_demonstrates_out_of_order_delivery() {
        let mut last = [0; 5];
        let mut reordered = [0; 5];
        let mut arrivals: Vec<_> = (0..120)
            .map(|i| {
                let (event, delay) = input(i, Motion::default());
                (event.time.0 + delay * 1_000_000, event)
            })
            .collect();
        arrivals.sort_by_key(|(due, _)| *due);
        for (_, event) in arrivals {
            let client = event.client_id as usize - 1;
            reordered[client] += usize::from(event.time.0 < last[client]);
            last[client] = last[client].max(event.time.0);
        }
        assert!(reordered.iter().all(|&n| n >= 2), "{reordered:?}");
    }
}
