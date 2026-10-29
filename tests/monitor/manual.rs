use crate::support::{self, Change, Time};
use contime::monitor::{ApplicationActivity, WorkerReading};
use std::time::Duration;

#[test]
fn manually_collected_status_feeds_the_same_activity_api_without_advancing_work() {
    let mut engine = support::manual();
    let before = WorkerReading { started_at: Duration::ZERO, completed_at: Duration::ZERO, workers: vec![engine.worker_status()] };
    engine.apply([Change { id: 1, at: Time(1), targets: vec![1, 2], delta: 3 }]).unwrap();
    engine.advance_to(Time(1)).unwrap();
    let queued = engine.queued();
    assert_eq!(engine.worker_status().counters.unwrap().inserted_applications, 0);
    assert_eq!(engine.queued(), queued);
    support::drive(&mut engine);
    let after =
        WorkerReading { started_at: Duration::from_secs(1), completed_at: Duration::from_secs(1), workers: vec![engine.worker_status()] };
    let activity = ApplicationActivity::between(Some(&before), &after);
    assert_eq!(activity.inserted_applications, Some(2));
    assert_eq!(activity.replay_applications, Some(2));
    assert_eq!(activity.inserted_per_second, Some(2.0));
    assert_eq!(activity.replay_per_second, Some(2.0));
}
