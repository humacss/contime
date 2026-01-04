use std::cmp::min;
use std::collections::VecDeque;

use crate::{ApplyEvent, Event, Snapshot};

use crate::{Checkpoint, index_before, ContimeKey};

#[inline(always)]
pub fn apply_event_in_place<S: Snapshot>(
    new_event: S::Event, 
    ordered_checkpoints: &mut VecDeque<Checkpoint<S>>, 
    ordered_events: &mut VecDeque<S::Event>,
    checkpoint_interval: usize,
) -> isize {

    let event_insert_index = index_before(ordered_events, ContimeKey { id: new_event.id(), time: new_event.time() }, |event: &S::Event| ContimeKey::from_event(event)).map_or(0, |index| index + 1);

    if event_insert_index < ordered_events.len() && ordered_events[event_insert_index].id() == new_event.id() {
        return 0;
    }

    let mut checkpoint_index = index_before(ordered_checkpoints, ContimeKey { id: new_event.snapshot_id(), time: new_event.time() }, |checkpoint: &Checkpoint<S>| ContimeKey::from_snapshot(&checkpoint.snapshot)).unwrap_or(1);
    
    if event_insert_index == ordered_events.len() {
        ordered_events.push_back(new_event);
    }else if event_insert_index == 0 {
        ordered_events.push_front(new_event);
    }else{
        ordered_events.insert(event_insert_index, new_event);    
    }

    let (first, second) = ordered_events.as_slices();
    let first_index = min(ordered_checkpoints[checkpoint_index].next_event_index, first.len());
    let second_index = ordered_checkpoints[checkpoint_index].next_event_index.saturating_sub(first.len());

    for event in &first[first_index..] {
        if ordered_checkpoints[checkpoint_index].event_count >= checkpoint_interval {
            if checkpoint_index+1 == ordered_checkpoints.len() {
                ordered_checkpoints.push_back(ordered_checkpoints[checkpoint_index].clone());    
            }else{
                ordered_checkpoints[checkpoint_index+1] = ordered_checkpoints[checkpoint_index].clone(); 
            }

            checkpoint_index += 1;
            ordered_checkpoints[checkpoint_index].event_count = 0;
        }

        event.apply_to(&mut ordered_checkpoints[checkpoint_index].snapshot);

        ordered_checkpoints[checkpoint_index].snapshot.set_time(event.time());
        ordered_checkpoints[checkpoint_index].event_count += 1;
        ordered_checkpoints[checkpoint_index].next_event_index += 1;
    }

    for event in &second[second_index..] {
        if ordered_checkpoints[checkpoint_index].event_count >= checkpoint_interval {
            if checkpoint_index+1 == ordered_checkpoints.len() {
                ordered_checkpoints.push_back(ordered_checkpoints[checkpoint_index].clone());    
            }else{
                ordered_checkpoints[checkpoint_index+1] = ordered_checkpoints[checkpoint_index].clone(); 
            }
            
            checkpoint_index += 1;
            ordered_checkpoints[checkpoint_index].event_count = 0;
        }

        event.apply_to(&mut ordered_checkpoints[checkpoint_index].snapshot);

        ordered_checkpoints[checkpoint_index].snapshot.set_time(event.time());
        ordered_checkpoints[checkpoint_index].event_count += 1;
        ordered_checkpoints[checkpoint_index].next_event_index += 1;
    }    

    0
}

#[cfg(test)]
mod tests {
    use super::*;
    
    use crate::{TestEvent, TestSnapshot};
    use rstest::rstest;

    type Value = i16;
    type Time = i64;
    type Sum = i32;
    type Items = Vec<i16>;
    const CHECKPOINT_EVENT_COUNT: usize = 2;

    fn gen_event(time: Time, value: Value) -> TestEvent {
        let snapshot_id = 0;

        if value >= 0 {
            TestEvent::Positive(snapshot_id, time, time.try_into().unwrap(), value.abs() as u16)
        }else{
            TestEvent::Negative(snapshot_id, time, time.try_into().unwrap(), value.abs() as u16)
        }
    }

    fn gen_checkpoint(index: usize, time: Time, sum: Sum, next_event_index: usize, items: Items) -> Checkpoint<TestSnapshot> {
        let snapshot_id = 0;
        let mut event_count = items.len() % CHECKPOINT_EVENT_COUNT;
        if event_count == 0 && items.len() > 0 {
            event_count = CHECKPOINT_EVENT_COUNT;
        }

        if index == 0 {
            event_count = CHECKPOINT_EVENT_COUNT;
        }

        Checkpoint { snapshot: TestSnapshot { id: snapshot_id, time, sum, items: items }, event_count, next_event_index }
    }

    #[rstest]
    #[case::empty(
        (1, 1), 
        vec![(0, 0, 0, vec![]), (0, 0, 0, vec![])],
        vec![],

        vec![(0, 0, 0, vec![]), (1, 1, 1, vec![1])], 
        vec![(1, 1)],
    )]
    #[case::in_order(
        (2, 2), 
        vec![(0, 0, 0, vec![]), (1, 1, 1, vec![1])],
        vec![(1, 1)],
        vec![(0, 0, 0, vec![]), (2, 3, 2, vec![1, 2])], 
        vec![(1, 1), (2, 2)],
    )]
    #[case::out_of_order(
       (1, 1), 
       vec![(0, 0, 0, vec![]), (1, 2, 1, vec![2])],
       vec![(2, 2)],

       vec![(0, 0, 0, vec![]), (2, 3, 2, vec![1, 2])], 
       vec![(1, 1), (2, 2)],
    )]
    #[case::duplicate_event(
       (1, 2), 
       vec![(0, 0, 0, vec![]), (1, 1, 1, vec![1])],
       vec![(1, 1)],

       vec![(0, 0, 0, vec![]), (1, 1, 1, vec![1])], 
       vec![(1, 1)],
    )]
    fn test_apply_event_in_place(
        #[case] new_event: (Time, Value), 
        #[case] ordered_checkpoints: Vec<(Time, Sum, usize, Items)>, 
        #[case] ordered_events: Vec<(Time, Value)>,

        #[case] expected_checkpoints: Vec<(Time, Sum, usize, Items)>, 
        #[case] expected_events: Vec<(Time, Value)>,
    ) { 
        let expected_events: VecDeque<TestEvent> = expected_events.into_iter().map(|(time, value)| gen_event(time, value)).collect();
        let expected_checkpoints: VecDeque<Checkpoint<TestSnapshot>> = expected_checkpoints.into_iter().enumerate().map(|(i, (time, sum, next_event_index, items))| gen_checkpoint(i, time, sum, next_event_index, items)).collect();

        let new_event = gen_event(new_event.0, new_event.1);
        let mut ordered_events: VecDeque<TestEvent> = ordered_events.into_iter().map(|(time, value)| gen_event(time, value)).collect();
        let mut ordered_checkpoints: VecDeque<Checkpoint<TestSnapshot>> = ordered_checkpoints.into_iter().enumerate().map(|(i, (time, sum, next_event_index, items))| gen_checkpoint(i, time, sum, next_event_index, items)).collect();

        apply_event_in_place(
            new_event, 
            &mut ordered_checkpoints,
            &mut ordered_events,
            CHECKPOINT_EVENT_COUNT
        );

        assert_eq!(expected_events, ordered_events);
        assert_eq!(expected_checkpoints, ordered_checkpoints);
    }
}