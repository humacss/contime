use std::time::Duration;

use contime::{TestEvent, TestSnapshot, TestSnapshotContime};

#[test]
fn test_reconciliation_from_out_of_order_event() {
    let c = TestSnapshotContime::new(1, 100000);

    // Add in-order event first
    c.apply_event(TestEvent::Positive(1, 10, 10, 10)).unwrap();

    // Query to get the reconciliation rx
    let (_, reconciliation_rx) = c.at::<TestSnapshot>(11, 1).unwrap();

    // Now add out-of-order event
    c.apply_event(TestEvent::Positive(1, 5, 5, 20)).unwrap();

    // Should receive reconciliation notification
    let recon = reconciliation_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(recon.snapshot_id, 1);
    assert_eq!(recon.from_time, 5);
    assert_eq!(recon.to_time, 10);
}

#[test]
fn test_reconciliation_from_snapshot() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 5, 5, 10)).unwrap();
    c.apply_event(TestEvent::Positive(1, 10, 10, 20)).unwrap();

    // Query to get the reconciliation rx
    let (_, reconciliation_rx) = c.at::<TestSnapshot>(11, 1).unwrap();

    // Apply authoritative snapshot at t=3 — earlier than existing events
    let auth = TestSnapshot { id: 1, time: 3, sum: 100, items: vec![] };
    c.apply_snapshot(auth).unwrap();

    let recon = reconciliation_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(recon.snapshot_id, 1);
    assert_eq!(recon.from_time, 3);
    assert_eq!(recon.to_time, 10);
}

#[test]
fn test_no_reconciliation_in_order() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 10)).unwrap();

    let (_, reconciliation_rx) = c.at::<TestSnapshot>(2, 1).unwrap();

    // Apply events strictly in order
    c.apply_event(TestEvent::Positive(1, 3, 3, 20)).unwrap();
    c.apply_event(TestEvent::Positive(1, 5, 5, 30)).unwrap();

    // No reconciliation should be sent
    assert!(reconciliation_rx.recv_timeout(Duration::from_millis(100)).is_err(), "expected no reconciliation for in-order events");
}

#[test]
fn test_multiple_reconciliations() {
    let c = TestSnapshotContime::new(1, 100000);

    // Establish a high-water mark
    c.apply_event(TestEvent::Positive(1, 100, 100, 1)).unwrap();

    let (_, reconciliation_rx) = c.at::<TestSnapshot>(101, 1).unwrap();

    // Send multiple out-of-order events
    c.apply_event(TestEvent::Positive(1, 50, 50, 1)).unwrap();
    c.apply_event(TestEvent::Positive(1, 30, 30, 1)).unwrap();
    c.apply_event(TestEvent::Positive(1, 10, 10, 1)).unwrap();

    // Should receive multiple reconciliation notifications
    let mut count = 0;
    while reconciliation_rx.recv_timeout(Duration::from_secs(2)).is_ok() {
        count += 1;
    }

    assert!(count >= 3, "expected at least 3 reconciliations, got {}", count);
}

#[test]
fn test_apply_snapshot_only_replays_strictly_later_events() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(10, 5, 1, 10)).unwrap();
    c.apply_event(TestEvent::Positive(10, 5, 20, 20)).unwrap();
    c.apply_event(TestEvent::Positive(10, 6, 30, 30)).unwrap();

    let auth = TestSnapshot { id: 10, time: 5, sum: 100, items: vec![] };
    c.apply_snapshot(auth).unwrap();

    let (snap, _) = c.at::<TestSnapshot>(6, 10).unwrap();
    assert_eq!(snap.sum, 100);

    let (snap, _) = c.at::<TestSnapshot>(7, 10).unwrap();
    assert_eq!(snap.sum, 130);
}

#[test]
fn test_reconciliation_is_broadcast_to_multiple_queries() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 10, 10, 10)).unwrap();

    let (_, rx_a) = c.at::<TestSnapshot>(11, 1).unwrap();
    let (_, rx_b) = c.at::<TestSnapshot>(11, 1).unwrap();

    c.apply_event(TestEvent::Positive(1, 5, 5, 20)).unwrap();

    let recon_a = rx_a.recv_timeout(Duration::from_secs(2)).unwrap();
    let recon_b = rx_b.recv_timeout(Duration::from_secs(2)).unwrap();

    assert_eq!(recon_a.snapshot_id, 1);
    assert_eq!(recon_a.from_time, 5);
    assert_eq!(recon_a.to_time, 10);
    assert_eq!(recon_b, recon_a);
}
