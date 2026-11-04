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
        // Each outward leg is followed by an equal trip back to the center.
        // Keeping to the axes makes arrival-time drift easy to recognize.
        1 => [North, South, East, West, West, East, South, North, Stop, Stop][second as usize % 10],
        // Each bot stays in its own quadrant with a distinct square phase.
        2 => [East, South, West, North][second as usize % 4],
        3 => [South, East, North, West][second as usize % 4],
        4 => [West, North, East, South][second as usize % 4],
        _ => [North, West, South, East][second as usize % 4],
    };
    let time = Time(second * 1_000_000_000 + 1);
    let delay_ms = [1700, 150, 700, 1150, 1550, 200, 900, 450][(phase + client as usize * 2) % 8];
    (MovementInput { id: u128::from(index + 6), client_id: client.into(), time, direction, motion }, delay_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement_example::model::{Character, SECOND};
    #[test]
    fn every_bot_turns_on_the_same_one_second_tick() {
        for second in 0..10 {
            let times: Vec<_> = (1..5).map(|bot| input(second * 5 + bot, Motion::default()).0.time.0).collect();
            assert!(times.iter().all(|&time| time == second * 1_000_000_000 + 1), "{times:?}");
        }
    }
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
                    2 => (true, false),
                    3 => (false, false),
                    4 => (true, true),
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
    fn player_returns_to_center_on_each_axis_and_stays_clear_of_bot_routes() {
        let motion = Motion { client_spacing: 18, movement_speed: 8 };
        let mut states: Vec<_> = (1..=5).map(|id| Character::spawn(id, motion)).collect();
        for index in 0..600 {
            let (event, _) = input(index, motion);
            states[event.client_id as usize - 1].apply_input(&event);
            let player = states[0].position_at(event.time);
            assert!(player.iter().all(|coordinate| coordinate.abs() <= 8 * SECOND));
            assert!(player[0] == 0 || player[1] == 0, "intended movement stays on the central axes");
            for bot in &states[1..] {
                let position = bot.position_at(event.time);
                let separation = (player[0] - position[0]).abs().max((player[1] - position[1]).abs());
                assert!(separation >= 8 * SECOND, "intended routes must remain separate: {index}");
            }
            if event.client_id == 1 && index / 5 % 2 == 0 {
                assert_eq!(player, [0, 0], "each outward-and-back pair returns to the exact center");
            }
        }
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
