use std::time::Duration;

use contime::{SnapshotWakeCause, TestEvent, TestSnapshotContime, TestSnapshotLanes};

#[test]
fn in_order_event_emits_changed_wake() {
    let contime = TestSnapshotContime::new(2, 1_024);
    let wakes = contime.subscribe_wakes().expect("wake subscription should open");

    contime.apply_event(TestEvent::Positive(1, 5, 10, 3)).expect("event should apply");

    let wake = wakes.recv_timeout(Duration::from_secs(1)).expect("wake should arrive");

    assert_eq!(wake.snapshot_id, 1);
    assert_eq!(wake.from_time, 5);
    assert_eq!(wake.to_time, 5);
    assert_eq!(wake.cause, SnapshotWakeCause::Changed);

    match wake.lane {
        TestSnapshotLanes::TestSnapshot(snapshot) => {
            assert_eq!(snapshot.id, 1);
            assert_eq!(snapshot.time, 6);
            assert_eq!(snapshot.sum, 3);
        }
    }
}

#[test]
fn out_of_order_event_emits_reconciled_wake() {
    let contime = TestSnapshotContime::new(2, 1_024);
    let wakes = contime.subscribe_wakes().expect("wake subscription should open");

    contime.apply_event(TestEvent::Positive(1, 10, 10, 10)).expect("first event should apply");
    let _ = wakes.recv_timeout(Duration::from_secs(1));

    contime.apply_event(TestEvent::Positive(1, 5, 5, 20)).expect("late event should apply");

    let wake = wakes.recv_timeout(Duration::from_secs(1)).expect("reconciled wake should arrive");

    assert_eq!(wake.snapshot_id, 1);
    assert_eq!(wake.from_time, 5);
    assert_eq!(wake.to_time, 10);
    assert_eq!(wake.cause, SnapshotWakeCause::Reconciled);

    match wake.lane {
        TestSnapshotLanes::TestSnapshot(snapshot) => {
            assert_eq!(snapshot.id, 1);
            assert_eq!(snapshot.time, 11);
            assert_eq!(snapshot.sum, 30);
        }
    }
}

#[test]
fn authoritative_snapshot_emits_reconciled_wake() {
    let contime = TestSnapshotContime::new(2, 1_024);
    let wakes = contime.subscribe_wakes().expect("wake subscription should open");

    contime.apply_event(TestEvent::Positive(1, 10, 10, 10)).expect("first event should apply");
    let _ = wakes.recv_timeout(Duration::from_secs(1));

    contime.apply_snapshot(contime::TestSnapshot { id: 1, time: 5, sum: 42, items: vec![42] }).expect("snapshot should apply");

    let wake = wakes.recv_timeout(Duration::from_secs(1)).expect("snapshot wake should arrive");

    assert_eq!(wake.snapshot_id, 1);
    assert_eq!(wake.from_time, 5);
    assert_eq!(wake.to_time, 10);
    assert_eq!(wake.cause, SnapshotWakeCause::Reconciled);

    match wake.lane {
        TestSnapshotLanes::TestSnapshot(snapshot) => {
            assert_eq!(snapshot.id, 1);
            assert_eq!(snapshot.time, 11);
            assert_eq!(snapshot.sum, 52);
        }
    }
}

#[test]
fn advance_publishes_one_time_advance_after_completion() {
    let contime = TestSnapshotContime::new(2, 1_024);
    let advances = contime.subscribe_time_advances().expect("time subscription should open");

    contime.advance(10).expect("advance should succeed");

    let wake = advances.recv_timeout(Duration::from_secs(1)).expect("time advance should arrive");
    assert_eq!(wake.current_time, 10);
    assert_eq!(wake.delta, 10);
    assert!(advances.try_recv().is_err());
}

#[test]
fn advance_to_only_publishes_when_time_moves_forward() {
    let contime = TestSnapshotContime::new(2, 1_024);
    let advances = contime.subscribe_time_advances().expect("time subscription should open");

    contime.advance_to(50).expect("advance should succeed");
    contime.advance_to(40).expect("older time should be a no-op");
    contime.advance_to(50).expect("same time should be a no-op");

    let wake = advances.recv_timeout(Duration::from_secs(1)).expect("time advance should arrive");
    assert_eq!(wake.current_time, 50);
    assert_eq!(wake.delta, 50);
    assert!(advances.try_recv().is_err());
}
