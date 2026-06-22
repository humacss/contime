use contime::{Snapshot, TestEvent, TestSnapshot, TestSnapshotContime};

fn query_one(contime: &TestSnapshotContime, time: i64, snapshot_id: u128) -> Option<TestSnapshot> {
    contime.query_at(time, &[snapshot_id]).unwrap().pop().flatten().map(Into::into)
}

#[test]
fn test_query_not_found() {
    let c = TestSnapshotContime::new(1, 1000);

    assert_eq!(query_one(&c, 0, 999), None);
}

#[test]
fn test_query_at_various_times() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 2, 2, 10)).unwrap();
    c.apply_event(TestEvent::Positive(1, 5, 5, 20)).unwrap();
    c.apply_event(TestEvent::Positive(1, 8, 8, 30)).unwrap();

    assert_eq!(query_one(&c, 1, 1).unwrap().sum, 0);
    assert_eq!(query_one(&c, 3, 1).unwrap().sum, 10);
    assert_eq!(query_one(&c, 6, 1).unwrap().sum, 30);
    assert_eq!(query_one(&c, 9, 1).unwrap().sum, 60);
}

#[test]
fn test_query_before_any_events() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 10, 10, 50)).unwrap();

    let snap = query_one(&c, 0, 1).unwrap();
    assert_eq!(snap.sum, 0);
    assert_eq!(snap.items.len(), 0);
}

#[test]
fn test_query_at_exact_event_time() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 5, 5, 42)).unwrap();

    assert_eq!(query_one(&c, 5, 1).unwrap().sum, 42);
    assert_eq!(query_one(&c, 6, 1).unwrap().sum, 42);
}

#[test]
fn test_query_includes_all_same_time_events_independent_of_event_id_ordering() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(10, 5, 1, 40)).unwrap();
    c.apply_event(TestEvent::Positive(10, 5, 20, 60)).unwrap();

    assert_eq!(query_one(&c, 5, 10).unwrap().sum, 100);
    assert_eq!(query_one(&c, 6, 10).unwrap().sum, 100);
}

#[test]
fn test_query_multiple_snapshot_ids() {
    let c = TestSnapshotContime::new(4, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 10)).unwrap();
    c.apply_event(TestEvent::Positive(2, 1, 2, 20)).unwrap();
    c.apply_event(TestEvent::Positive(3, 1, 3, 30)).unwrap();

    let results = c.query_at(2, &[1, 2, 3]).unwrap();
    let snap1: TestSnapshot = results[0].clone().unwrap().into();
    let snap2: TestSnapshot = results[1].clone().unwrap().into();
    let snap3: TestSnapshot = results[2].clone().unwrap().into();

    assert_eq!(snap1.sum, 10);
    assert_eq!(snap1.id(), 1);
    assert_eq!(snap2.sum, 20);
    assert_eq!(snap2.id(), 2);
    assert_eq!(snap3.sum, 30);
    assert_eq!(snap3.id(), 3);
}

#[test]
fn test_query_at_returns_results_in_input_order_with_not_found_and_duplicates() {
    let c = TestSnapshotContime::new(4, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 10)).unwrap();
    c.apply_event(TestEvent::Positive(2, 1, 2, 20)).unwrap();
    c.apply_event(TestEvent::Positive(3, 1, 3, 30)).unwrap();

    let results = c.query_at(2, &[3, 999, 1, 2, 1]).unwrap();
    let sums = results
        .into_iter()
        .map(|lane| {
            lane.map(|lane| {
                let snapshot: TestSnapshot = lane.into();
                snapshot.sum
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(sums, vec![Some(30), None, Some(10), Some(20), Some(10)]);
}
