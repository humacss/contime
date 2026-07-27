use contime::{Event, TestEvent, TestSnapshotContime};

fn journal_keys(contime: &TestSnapshotContime) -> Vec<(i64, u128)> {
    contime.inspect_events(..).unwrap().into_iter().map(|entry| (entry.event.time(), entry.event.id())).collect()
}

#[test]
fn journal_orders_original_events_and_deduplicates_by_key() {
    let contime = TestSnapshotContime::with_history_horizon(2, 100_000, 50);

    contime
        .apply_events([TestEvent::Positive(3, 30, 300, 3), TestEvent::Positive(1, 10, 100, 1), TestEvent::Positive(2, 20, 200, 2)])
        .unwrap();
    contime.apply_events([TestEvent::Positive(2, 20, 200, 2)]).unwrap();

    assert_eq!(journal_keys(&contime), vec![(10, 100), (20, 200), (30, 300)]);

    let entries = contime.inspect_events(20..=30).unwrap();
    assert_eq!(entries.iter().map(|entry| (entry.event.time(), entry.event.id())).collect::<Vec<_>>(), vec![(20, 200), (30, 300)]);
    assert_eq!(entries[0].routed_snapshot_ids, vec![2]);
    assert_eq!(entries[1].routed_snapshot_ids, vec![3]);
}

#[test]
fn journal_records_enqueued_events() {
    let contime = TestSnapshotContime::with_history_horizon(2, 100_000, 50);

    contime.send_events([TestEvent::Positive(2, 25, 250, 4)]).unwrap();

    let entries = contime.inspect_events(25..=25).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].event.time(), 25);
    assert_eq!(entries[0].event.id(), 250);
    assert_eq!(entries[0].routed_snapshot_ids, vec![2]);
}

#[test]
fn journal_prunes_events_before_the_history_horizon() {
    let contime = TestSnapshotContime::with_history_horizon(2, 100_000, 50);
    contime
        .apply_events([TestEvent::Positive(1, 10, 100, 1), TestEvent::Positive(2, 20, 200, 2), TestEvent::Positive(3, 30, 300, 3)])
        .unwrap();

    contime.advance_to(70).unwrap();

    assert_eq!(journal_keys(&contime), vec![(20, 200), (30, 300)]);
}
