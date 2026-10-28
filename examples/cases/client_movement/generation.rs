//! The simulator creates direction changes, then assigns one bot client to each.
use crate::movement_example::model::{Direction, Motion, MovementInput, Time};
use contime::contime_simulator::{EventStream, StreamConfig};
pub fn events(clients: usize, start: u64, time: Time, motion: Motion) -> impl Iterator<Item = MovementInput> {
    let mut sequence = start;
    let mut client = 1;
    EventStream::new(
        StreamConfig { event_count: clients.saturating_sub(1) as u64, seed: start ^ 42, settings: () },
        move |_, random| {
            sequence += 1;
            MovementInput { id: sequence.into(), client_id: 0, time, direction: Direction::from_index(random.next_u64()), motion }
        },
        move |_, _, event: &mut MovementInput| {
            client += 1;
            event.client_id = client;
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bots_have_unique_ids_and_leave_client_one_for_keyboard_input() {
        let events: Vec<_> = events(4, 10, Time(100), Motion::default()).collect();
        assert_eq!(
            events.iter().map(|e| (e.id, e.client_id, e.time)).collect::<Vec<_>>(),
            [(11, 2, Time(100)), (12, 3, Time(100)), (13, 4, Time(100))]
        );
        assert_eq!(super::events(1, 0, Time(1), Motion::default()).count(), 0);
    }
}
