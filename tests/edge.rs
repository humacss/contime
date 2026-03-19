use contime::{TestSnapshotContime, TestEvent, TestSnapshot, Snapshot};

#[test]
fn test_negative_event_application() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 50)).unwrap();
    c.apply_event(TestEvent::Negative(1, 2, 2, 20)).unwrap();
    c.apply_event(TestEvent::Negative(1, 3, 3, 10)).unwrap();

    let (snap, _) = c.at::<TestSnapshot>(4, 1).unwrap();
    assert_eq!(snap.sum, 20); // 50 - 20 - 10
    assert_eq!(snap.items, vec![50, -20, -10]);
}

#[test]
fn test_duplicate_event_ids_different_times() {
    let c = TestSnapshotContime::new(1, 100000);

    // Same event id (0) but different times — BTreeMap keys differ by time
    c.apply_event(TestEvent::Positive(1, 1, 0, 10)).unwrap();
    c.apply_event(TestEvent::Positive(1, 5, 0, 20)).unwrap();

    let (snap, _) = c.at::<TestSnapshot>(6, 1).unwrap();
    assert_eq!(snap.sum, 30); // both events applied
}

#[test]
fn test_many_snapshots_many_workers() {
    let c = TestSnapshotContime::new(8, 1000000);

    // Create 20 independent snapshots with different events
    for snap_id in 1..=20u128 {
        for i in 0..5u128 {
            c.apply_event(TestEvent::Positive(snap_id, i as i64, snap_id * 1000 + i, snap_id as u16)).unwrap();
        }
    }

    // Verify each snapshot independently
    for snap_id in 1..=20u128 {
        let (snap, _) = c.at::<TestSnapshot>(5, snap_id).unwrap();
        assert_eq!(snap.id(), snap_id);
        assert_eq!(snap.sum, (snap_id as i32) * 5, "snapshot {} has wrong sum", snap_id);
    }
}

#[test]
fn test_interleaved_events_across_snapshots() {
    let c = TestSnapshotContime::new(4, 100000);

    // Interleave events for two different snapshots
    c.apply_event(TestEvent::Positive(1, 1, 1, 10)).unwrap();
    c.apply_event(TestEvent::Positive(2, 1, 2, 100)).unwrap();
    c.apply_event(TestEvent::Positive(1, 2, 3, 20)).unwrap();
    c.apply_event(TestEvent::Positive(2, 2, 4, 200)).unwrap();

    let (snap1, _) = c.at::<TestSnapshot>(3, 1).unwrap();
    let (snap2, _) = c.at::<TestSnapshot>(3, 2).unwrap();

    // Each snapshot should only have its own events
    assert_eq!(snap1.sum, 30);
    assert_eq!(snap2.sum, 300);
}

#[test]
fn test_snapshot_time_is_set_on_query() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 5)).unwrap();

    // Query at t=50 — the returned snapshot's time should be 50
    let (snap, _) = c.at::<TestSnapshot>(50, 1).unwrap();
    assert_eq!(snap.time(), 50);
    assert_eq!(snap.sum, 5);
}

#[test]
fn test_out_of_order_events_deterministic() {
    let c = TestSnapshotContime::new(1, 100000);

    // Apply events out of order
    c.apply_event(TestEvent::Positive(1, 10, 10, 10)).unwrap();
    c.apply_event(TestEvent::Positive(1, 5, 5, 20)).unwrap();
    c.apply_event(TestEvent::Positive(1, 1, 1, 5)).unwrap();
    c.apply_event(TestEvent::Negative(1, 7, 7, 3)).unwrap();

    // Query at various times and verify correctness
    let (snap, _) = c.at::<TestSnapshot>(2, 1).unwrap();
    assert_eq!(snap.sum, 5); // only event at t=1 applied

    let (snap, _) = c.at::<TestSnapshot>(6, 1).unwrap();
    assert_eq!(snap.sum, 25); // t=1 (+5) + t=5 (+20)

    let (snap, _) = c.at::<TestSnapshot>(8, 1).unwrap();
    assert_eq!(snap.sum, 22); // t=1 (+5) + t=5 (+20) + t=7 (-3)

    let (snap, _) = c.at::<TestSnapshot>(11, 1).unwrap();
    assert_eq!(snap.sum, 32); // +5 +20 -3 +10
}
