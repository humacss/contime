use contime::{ContimeError, Event, QueryEventsResult, TestEvent, TestEventLanes, TestSnapshot, TestSnapshotContime};

fn positive(snapshot_id: u128, time: i64, event_id: u128, value: u16) -> TestEvent {
    TestEvent::Positive(snapshot_id, time, event_id, value)
}

fn negative(snapshot_id: u128, time: i64, event_id: u128, value: u16) -> TestEvent {
    TestEvent::Negative(snapshot_id, time, event_id, value)
}

fn event_ids(events: &[TestEventLanes]) -> Vec<u128> {
    events.iter().map(Event::id).collect()
}

#[test]
fn events_between_unknown_snapshot_returns_not_found() {
    let c = TestSnapshotContime::new(1, 100000);

    match c.events_between(999, 0, 10) {
        Err(ContimeError::NotFound) => {}
        Ok(events) => panic!("expected NotFound, got {events:?}"),
        Err(err) => panic!("unexpected error: {err:?}"),
    }
}

#[test]
fn query_events_between_unknown_snapshot_returns_not_found() {
    let c = TestSnapshotContime::new(1, 100000);

    match c.query_events_between(999, 0, 10).unwrap().wait().unwrap() {
        QueryEventsResult::NotFound => {}
        QueryEventsResult::Found(events) => panic!("expected NotFound, got {events:?}"),
    }
}

#[test]
fn events_between_known_snapshot_returns_empty_when_range_has_no_events() {
    let c = TestSnapshotContime::new(1, 100000);
    c.apply_event(positive(1, 10, 10, 5)).unwrap();

    assert!(c.events_between(1, 0, 10).unwrap().is_empty());
    assert!(c.events_between(1, 11, 20).unwrap().is_empty());
    assert!(c.events_between(1, 20, 20).unwrap().is_empty());
}

#[test]
fn events_between_uses_half_open_time_range() {
    let c = TestSnapshotContime::new(1, 100000);
    c.apply_event(positive(1, 4, 40, 4)).unwrap();
    c.apply_event(positive(1, 5, 50, 5)).unwrap();
    c.apply_event(positive(1, 6, 60, 6)).unwrap();

    let events = c.events_between(1, 5, 6).unwrap();

    assert_eq!(event_ids(&events), vec![50]);
}

#[test]
fn events_between_orders_same_time_events_by_event_id() {
    let c = TestSnapshotContime::new(1, 100000);
    c.apply_event(positive(1, 5, 30, 3)).unwrap();
    c.apply_event(positive(1, 5, 10, 1)).unwrap();
    c.apply_event(positive(1, 5, 20, 2)).unwrap();

    let events = c.events_between(1, 5, 6).unwrap();

    assert_eq!(event_ids(&events), vec![10, 20, 30]);
}

#[test]
fn events_between_orders_out_of_order_applies_by_time_then_event_id() {
    let c = TestSnapshotContime::new(1, 100000);
    c.apply_event(positive(1, 10, 10, 10)).unwrap();
    c.apply_event(positive(1, 5, 50, 5)).unwrap();
    c.apply_event(negative(1, 7, 70, 7)).unwrap();

    let events = c.events_between(1, 0, 11).unwrap();

    assert_eq!(event_ids(&events), vec![50, 70, 10]);
}

#[test]
fn events_between_returns_duplicate_event_once() {
    let c = TestSnapshotContime::new(1, 100000);
    c.apply_event(positive(1, 5, 50, 5)).unwrap();
    c.apply_event(positive(1, 5, 50, 5)).unwrap();

    let events = c.events_between(1, 0, 10).unwrap();

    assert_eq!(event_ids(&events), vec![50]);
}

#[test]
fn events_between_routes_to_requested_snapshot_only_across_workers() {
    let c = TestSnapshotContime::new(4, 100000);
    c.apply_event(positive(1, 1, 10, 10)).unwrap();
    c.apply_event(positive(2, 1, 20, 20)).unwrap();
    c.apply_event(positive(3, 1, 30, 30)).unwrap();
    c.apply_event(positive(1, 2, 11, 11)).unwrap();

    let events = c.events_between(1, 0, 10).unwrap();

    assert_eq!(event_ids(&events), vec![10, 11]);
}

#[test]
fn query_events_between_handle_matches_blocking_query() {
    let c = TestSnapshotContime::new(1, 100000);
    c.apply_event(positive(1, 1, 10, 10)).unwrap();
    c.apply_event(positive(1, 2, 20, 20)).unwrap();

    let blocking = c.events_between(1, 0, 10).unwrap();
    let handled = match c.query_events_between(1, 0, 10).unwrap().wait().unwrap() {
        QueryEventsResult::Found(events) => events,
        QueryEventsResult::NotFound => panic!("expected found event history"),
    };

    assert_eq!(handled, blocking);
}

#[test]
fn events_between_returns_only_retained_history_after_horizon_pruning() {
    let c = TestSnapshotContime::with_history_horizon(1, 100000, 10);
    c.apply_event(positive(1, 1, 10, 10)).unwrap();
    c.apply_event(positive(1, 5, 50, 50)).unwrap();
    c.apply_event(positive(1, 20, 200, 200)).unwrap();

    c.advance_to(16).unwrap();

    let events = c.events_between(1, 0, 30).unwrap();

    assert_eq!(event_ids(&events), vec![200]);
}

#[test]
fn events_between_read_does_not_mutate_snapshot_history() {
    let c = TestSnapshotContime::new(1, 100000);
    c.apply_event(positive(1, 1, 10, 10)).unwrap();

    let before: TestSnapshot = c.at(2, 1).unwrap().0;
    let _events = c.events_between(1, 0, 10).unwrap();
    let after: TestSnapshot = c.at(2, 1).unwrap().0;

    assert_eq!(after, before);
}
