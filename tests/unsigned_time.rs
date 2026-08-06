//! Verifies that primitive unsigned time supports history horizons without arithmetic overflow.

use contime::{ContimeEvent, ContimeSnapshot};

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.id, time = self.time, time_type = u64, bytes = 32)]
pub struct ValueChanged {
    id: u128,
    snapshot_id: u128,
    time: u64,
    value: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [ValueChanged],
    id = [snapshot_id],
    time_type = u64,
    bytes = 32,
    apply = {
        for event in batch.events {
            match event {
                ValueAtEvent::ValueChanged(event) => self.value = event.value,
            }
        }
    }
)]
pub struct ValueAt {
    snapshot_id: u128,
    time: u64,
    value: i32,
}

contime::lanes! {
    mod value_lanes;
    time u64;
    snapshots [ValueAt];
    routes [
        ValueChanged => [ValueAt],
    ];
}

#[test]
fn unsigned_time_saturates_a_horizon_larger_than_current_time() {
    let contime = value_lanes::Contime::with_history_horizon(1, 1_000_000, 10);

    contime
        .apply([ValueChanged { id: 1, snapshot_id: 7, time: 0, value: 42 }].map(Into::into))
        .expect("an unsigned event at zero should remain valid while the horizon exceeds current time");
    contime.advance_to(5).expect("advancing unsigned time should safely calculate a horizon that saturates at zero");

    let snapshot: ValueAt = contime
        .query_at(5, &[7])
        .expect("the retained unsigned-time snapshot should remain queryable")
        .pop()
        .flatten()
        .expect("the retained unsigned-time snapshot should exist")
        .into();

    assert_eq!(snapshot.value, 42, "safe unsigned horizon arithmetic should retain events at the saturated zero boundary");
}
