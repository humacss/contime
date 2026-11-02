//! Simulator callbacks preserve transaction identity across identical retries.
use crate::accounts_example::model::Transaction;
use crate::definition::Time;
use contime::contime_simulator::{EventStream, StreamConfig};
pub fn transaction(id: u128, time: Time, amount: i64) -> Transaction {
    EventStream::new(
        StreamConfig { event_count: 1, seed: id as u64, settings: () },
        move |_, _| Transaction { id, account_id: 0, time, amount },
        |_, _, event: &mut Transaction| event.account_id = 1,
    )
    .next()
    .expect("one configured transaction")
}
