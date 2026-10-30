//! Canonical reference timelines for delivered inputs, bounded by retention.
use crate::movement_example::model::{Character, Motion, MovementInput, Time};
use std::collections::BTreeMap;
struct Timeline {
    anchor: Character,
    inputs: BTreeMap<(Time, u128), MovementInput>,
}
pub struct Reference {
    timelines: Vec<Timeline>,
    motion: Motion,
}
impl Reference {
    pub fn new(clients: usize, motion: Motion) -> Self {
        Self {
            motion,
            timelines: (1..=clients).map(|id| Timeline { anchor: Character::spawn(id as u128, motion), inputs: BTreeMap::new() }).collect(),
        }
    }
    pub fn initial_states(&self) -> Vec<Character> {
        self.timelines.iter().map(|timeline| Character::spawn(timeline.anchor.client_id, self.motion)).collect()
    }
    pub fn insert(&mut self, event: MovementInput) {
        self.timelines[event.client_id as usize - 1].inputs.insert((event.time, event.id), event);
    }
    pub fn states(&self, time: Time) -> Vec<Character> {
        self.timelines
            .iter()
            .map(|timeline| {
                let mut state = timeline.anchor.clone();
                for event in timeline.inputs.values().take_while(|e| e.time <= time) {
                    state.apply_input(event);
                }
                state
            })
            .collect()
    }
    /// Exact historical location of a delivered input, after canonical ordering.
    pub fn position(&self, client: usize, time: Time) -> [i128; 2] {
        let timeline = &self.timelines[client - 1];
        let mut state = timeline.anchor.clone();
        for event in timeline.inputs.values().take_while(|e| e.time <= time) {
            state.apply_input(event);
        }
        state.position_at(time)
    }
    pub fn compact(&mut self, before: Time) {
        for timeline in &mut self.timelines {
            let keys: Vec<_> = timeline.inputs.range(..(before, 0)).map(|(key, _)| *key).collect();
            for key in keys {
                let event = timeline.inputs.remove(&key).expect("selected reference input");
                timeline.anchor.apply_input(&event);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement_example::model::{Direction, SECOND};
    #[test]
    fn late_turn_repairs_a_materialized_path_and_compaction_preserves_the_prefix() {
        let mut reference = Reference::new(1, Motion::default());
        let input = |id, seconds: u64, direction| MovementInput {
            id,
            client_id: 1,
            time: Time(seconds * 1_000_000_000),
            direction,
            motion: Motion::default(),
        };
        reference.insert(input(1, 0, Direction::East));
        reference.insert(input(3, 2, Direction::Stop));
        assert_eq!(reference.states(Time(3_000_000_000))[0].position_at(Time(3_000_000_000)), [12 * SECOND, 0]);
        reference.insert(input(2, 1, Direction::North));
        reference.compact(Time(2_000_000_000));
        assert_eq!(reference.states(Time(3_000_000_000))[0].position_at(Time(3_000_000_000)), [6 * SECOND, 6 * SECOND]);
    }
}
