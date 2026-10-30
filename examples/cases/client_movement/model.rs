//! Timestamped direction changes and one independently reconstructed character.
pub use crate::definition::Time;
#[cfg(test)]
use contime::checkpoints::{ApplyEvents, Snapshot};
use contime::{ContimeEvent, ContimeSnapshot};
pub const SECOND: i128 = 1_000_000_000;
/// Fixed arena half-size in the same fixed-point coordinates as positions.
pub const ARENA_HALF_EXTENT: [i128; 2] = [48 * SECOND, 28 * SECOND];
/// Per-run motion values carried by inputs so snapshot creation needs no global configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Motion {
    pub client_spacing: u64,
    pub movement_speed: u64,
}
impl Default for Motion {
    fn default() -> Self {
        Self { client_spacing: 4, movement_speed: 6 }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// Persistent movement selection, until the next timestamped input changes it.
pub enum Direction {
    #[default]
    Stop,
    North,
    East,
    South,
    West,
}
impl Direction {
    pub fn from_index(index: u64) -> Self {
        match index % 5 {
            1 => Self::North,
            2 => Self::East,
            3 => Self::South,
            4 => Self::West,
            _ => Self::Stop,
        }
    }
    pub fn velocity(self, speed: u64) -> [i128; 2] {
        let speed = i128::from(speed);
        match self {
            Self::Stop => [0, 0],
            Self::North => [0, speed],
            Self::East => [speed, 0],
            Self::South => [0, -speed],
            Self::West => [-speed, 0],
        }
    }
}
/// One client changes direction at its original input timestamp.
#[derive(Clone, Debug, ContimeEvent)]
#[contime_event(id = |event| event.id)]
pub struct MovementInput {
    /// Stable event identity; retained retransmissions reuse it.
    pub id: u128,
    /// Destination snapshot ID, one per client. Client 1 belongs to the keyboard.
    pub client_id: u128,
    /// Original input time, preserved through simulated network delivery.
    pub time: Time,
    /// Direction selected at this time.
    pub direction: Direction,
    /// Identical for all inputs in this run; used when creating or replaying a snapshot.
    pub motion: Motion,
}
/// Position is stored in billionths of a terminal world unit, without float drift.
#[derive(Debug, ContimeSnapshot)]
#[contime_snapshot(
    id = |client_id| *client_id,
    events = [MovementInput],
    create = |_id, event| Self::spawn(event.client_id, event.motion),
    apply = {
        for event in batch.events {
            let CharacterEvent::MovementInput(input) = event;
            self.apply_input(input);
        }
    }
)]
pub struct Character {
    /// Destination snapshot ID, one per client. Client 1 belongs to the keyboard.
    pub client_id: u128,
    /// ConTime snapshot metadata; independent of the movement integration anchor.
    pub time: Time,
    /// Position at `integrated_at`, in fixed-point world coordinates.
    pub position: [i128; 2],
    /// Separate from Snapshot time: queries and pruning may update that metadata.
    pub integrated_at: Time,
    pub direction: Direction,
    /// Identical for all inputs in this run; used when creating or replaying a snapshot.
    pub motion: Motion,
    /// Input applications represented by this snapshot, used for validation.
    pub events: u64,
}
impl Character {
    pub fn spawn(client_id: u128, motion: Motion) -> Self {
        let index = client_id.saturating_sub(1);
        let spacing = i128::from(motion.client_spacing) * SECOND;
        let columns = if spacing == 0 { 1 } else { (2 * (ARENA_HALF_EXTENT[0] / spacing) + 1) as u128 };
        // Alternate around the origin and wrap excess clients onto the finite grid.
        let coordinate = |index: u128, half: i128| {
            if spacing == 0 {
                return 0;
            }
            let index = (index % (2 * (half / spacing) + 1) as u128) as i128;
            let cell = if index % 2 == 1 { (index + 1) / 2 } else { -(index / 2) };
            cell * spacing
        };
        Self {
            client_id,
            motion,
            position: [coordinate(index % columns, ARENA_HALF_EXTENT[0]), coordinate(index / columns, ARENA_HALF_EXTENT[1])],
            ..Self::default()
        }
    }
    /// Position implied by the accepted input history at a requested time.
    pub fn position_at(&self, time: Time) -> [i128; 2] {
        let elapsed = i128::from(time.0.saturating_sub(self.integrated_at.0));
        let velocity = self.direction.velocity(self.motion.movement_speed);
        std::array::from_fn(|axis| {
            (self.position[axis] + velocity[axis] * elapsed).clamp(-ARENA_HALF_EXTENT[axis], ARENA_HALF_EXTENT[axis])
        })
    }
    /// Integrate the old direction first, then apply the new direction.
    pub fn apply_input(&mut self, event: &MovementInput) {
        self.position = self.position_at(event.time);
        self.integrated_at = event.time;
        self.direction = event.direction;
        self.motion = event.motion;
        self.events += 1;
    }
}
crate::definition::single_lane!(MovementInput => Character);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn turns_integrate_the_previous_direction_and_snapshot_metadata_does_not_move_the_anchor() {
        let mut character = Character::spawn(1, Motion::default());
        let input = |id, t, direction| MovementInput { id, client_id: 1, time: Time(t), direction, motion: Motion::default() };
        character.apply_input(&input(1, 0, Direction::East));
        character.apply_input(&input(2, 1_000_000_000, Direction::North));
        character.set_time(Time(9_000_000_000));
        assert_eq!(character.position_at(Time(2_000_000_000)), [6 * SECOND, 6 * SECOND]);
        character.apply_input(&input(3, 2_000_000_000, Direction::Stop));
        assert_eq!(character.position_at(Time(10_000_000_000)), [6 * SECOND, 6 * SECOND]);
        assert_eq!(character.events, 3);
    }
    #[test]
    fn configured_motion_initializes_the_origin_and_preserves_old_velocity_at_turns() {
        let motion = Motion { client_spacing: 9, movement_speed: 10 };
        let input = |id, t, direction| MovementInput { id, client_id: 2, time: Time(t), direction, motion };
        let east = input(1, 0, Direction::East);
        let mut state = Character::create(2, &east);
        state.apply_input(&east);
        state.apply_input(&input(2, 1_000_000_000, Direction::North));
        state.apply_input(&input(3, 2_000_000_000, Direction::Stop));
        assert_eq!(state.position_at(Time(10_000_000_000)), [19 * SECOND, 10 * SECOND]);
    }
    #[test]
    fn characters_stop_at_each_wall_and_can_turn_back_inward() {
        let motion = Motion::default();
        let input =
            |id, seconds: u64, direction| MovementInput { id, client_id: 1, time: Time(seconds * 1_000_000_000), direction, motion };
        for (direction, axis, sign) in
            [(Direction::East, 0, 1), (Direction::West, 0, -1), (Direction::North, 1, 1), (Direction::South, 1, -1)]
        {
            let mut character = Character::spawn(1, motion);
            character.apply_input(&input(1, 0, direction));
            assert_eq!(character.position_at(Time(u64::MAX))[axis], sign * ARENA_HALF_EXTENT[axis]);
        }
        let mut character = Character::spawn(1, motion);
        character.apply_input(&input(1, 0, Direction::East));
        character.apply_input(&input(2, 100, Direction::West));
        assert_eq!(character.position_at(Time(101_000_000_000)), [ARENA_HALF_EXTENT[0] - 6 * SECOND, 0]);
    }
    #[test]
    fn every_spawn_is_inside_the_arena_even_with_extreme_spacing() {
        for spacing in [0, 4, 8, 9, 1_000_000] {
            let motion = Motion { client_spacing: spacing, ..Motion::default() };
            for id in 1..=10_000 {
                let character = Character::spawn(id, motion);
                assert!(character.position.iter().zip(ARENA_HALF_EXTENT).all(|(value, bound)| value.abs() <= bound));
            }
        }
        assert_eq!(Character::spawn(1, Motion::default()).position, [0, 0]);
    }
}
