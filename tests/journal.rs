use contime::{Input, TestEvent, TestSnapshotContime};

fn journal_keys(contime: &TestSnapshotContime) -> Vec<(i64, u128)> {
    contime.inspect_inputs(..).unwrap().into_iter().map(|entry| (entry.input.time(), entry.input.id())).collect()
}

#[test]
fn journal_orders_original_events_and_deduplicates_by_key() {
    let contime = TestSnapshotContime::with_history_horizon(2, 100_000, 50);

    contime
        .apply([TestEvent::Positive(3, 30, 300, 3), TestEvent::Positive(1, 10, 100, 1), TestEvent::Positive(2, 20, 200, 2)].map(Into::into))
        .unwrap();
    contime.apply([TestEvent::Positive(2, 20, 200, 2)].map(Into::into)).unwrap();

    assert_eq!(journal_keys(&contime), vec![(10, 100), (20, 200), (30, 300)]);

    let entries = contime.inspect_inputs(20..=30).unwrap();
    assert_eq!(entries.iter().map(|entry| (entry.input.time(), entry.input.id())).collect::<Vec<_>>(), vec![(20, 200), (30, 300)]);
    assert_eq!(entries[0].routed_snapshot_ids, vec![2]);
    assert_eq!(entries[1].routed_snapshot_ids, vec![3]);
}

#[test]
fn journal_records_enqueued_events() {
    let contime = TestSnapshotContime::with_history_horizon(2, 100_000, 50);

    contime.send([TestEvent::Positive(2, 25, 250, 4)].map(Into::into)).unwrap();

    let entries = contime.inspect_inputs(25..=25).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].input.time(), 25);
    assert_eq!(entries[0].input.id(), 250);
    assert_eq!(entries[0].routed_snapshot_ids, vec![2]);
}

#[test]
fn journal_prunes_events_before_the_history_horizon() {
    let contime = TestSnapshotContime::with_history_horizon(2, 100_000, 50);
    contime
        .apply([TestEvent::Positive(1, 10, 100, 1), TestEvent::Positive(2, 20, 200, 2), TestEvent::Positive(3, 30, 300, 3)].map(Into::into))
        .unwrap();

    contime.advance_to(70).unwrap();

    assert_eq!(journal_keys(&contime), vec![(20, 200), (30, 300)]);
}

#[test]
fn horizon_forgets_identity_and_allows_the_id_at_a_new_retained_time() {
    let contime = TestSnapshotContime::with_history_horizon(1, 100_000, 50);
    contime.apply([TestEvent::Positive(1, 10, 7, 10).into()]).unwrap();
    contime.apply([TestEvent::Positive(1, 11, 7, 99).into()]).unwrap();
    assert_eq!(contime.inspect_inputs(..).unwrap().len(), 1);

    contime.advance_to(70).unwrap();
    contime.apply([TestEvent::Positive(1, 30, 7, 20).into()]).unwrap();

    let retained = contime.inspect_inputs(20..).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!((retained[0].input.time(), retained[0].input.id()), (30, 7));
}
