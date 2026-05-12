use std::time::Duration;

use contime::TestSnapshotContime;

#[test]
fn advance_notifies_time_advance_subscribers() {
    let contime = TestSnapshotContime::new(1, 1_024);
    let advances = contime.subscribe_time_advances().expect("subscription should open");

    contime.advance(10).expect("advance should succeed");

    let advance = advances.recv_timeout(Duration::from_secs(1)).expect("time advance should arrive");
    assert_eq!(advance.current_time, 10);
    assert_eq!(advance.delta, 10);
}

#[test]
fn advance_to_notifies_time_advance_subscribers_once_for_forward_progress() {
    let contime = TestSnapshotContime::new(1, 1_024);
    let advances = contime.subscribe_time_advances().expect("subscription should open");

    contime.advance_to(50).expect("advance_to should succeed");
    contime.advance_to(25).expect("stale advance_to should be a no-op");

    let advance = advances.recv_timeout(Duration::from_secs(1)).expect("time advance should arrive");
    assert_eq!(advance.current_time, 50);
    assert_eq!(advance.delta, 50);
    assert!(advances.try_recv().is_err());
}
