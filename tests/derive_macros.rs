use contime::{ContimeEvent, ContimeSnapshot, Snapshot};

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 32)]
pub struct OnDerivedValueChanged {
    event_id: u128,
    entity_id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [OnDerivedValueChanged],
    id = [entity_id],
    time = self.time,
    bytes = 32,
    apply = {
        for event in batch.events {
            match event {
                DerivedValueAtEvent::OnDerivedValueChanged(event) => {
                    self.entity_id = event.entity_id;
                    self.value = event.value;
                }
            }
        }
        self.time = batch.time;
    }
)]
pub struct DerivedValueAt {
    entity_id: u128,
    time: i64,
    value: i32,
}

contime::lanes! {
    mod derived_lanes;
    snapshots [DerivedValueAt];
    routes [
        OnDerivedValueChanged => [DerivedValueAt],
    ];
}

#[test]
fn derives_generate_event_snapshot_and_lanes() {
    let contime = derived_lanes::Contime::new(1, 2_048);

    contime.apply_events([OnDerivedValueChanged { event_id: 10, entity_id: 7, time: 5, value: 99 }]).expect("event should apply");

    let snapshot: DerivedValueAt =
        contime.query_at(6, &[7]).expect("query should succeed").pop().flatten().expect("snapshot should exist").into();

    assert_eq!(7, snapshot.id());
    assert_eq!(99, snapshot.value);
}
