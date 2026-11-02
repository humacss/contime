//! Provisional ledger decisions, in integer credits. No external settlement occurs.
use crate::definition::Time;
use contime::prelude::*;
#[derive(Clone, Debug, ContimeEvent)]
pub struct Transaction {
    #[contime(id)]
    pub id: u128,
    pub account_id: u128,
    #[contime(time)]
    pub time: Time,
    /// Positive deposits; negative payment requests.
    pub amount: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub id: u128,
    pub time: Time,
    pub amount: i64,
    pub accepted: bool,
    pub balance: i64,
}
#[derive(Clone, Debug, Default, ContimeSnapshot)]
#[contime(events = [Transaction])]
pub struct Account {
    #[contime(id)]
    pub account_id: u128,
    #[contime(time)]
    pub time: Time,
    pub balance: i64,
    pub events: u64,
    pub decisions: Vec<Decision>,
}
impl Account {
    pub fn record(&mut self, event: &Transaction) {
        let accepted = event.amount >= 0 || self.balance >= -event.amount;
        if accepted {
            self.balance += event.amount;
        }
        self.events += 1;
        self.decisions.push(Decision { id: event.id, time: event.time, amount: event.amount, accepted, balance: self.balance });
        if self.decisions.len() > 64 {
            self.decisions.remove(0);
        }
    }
}
impl SnapshotApply for Account {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Transaction>() {
            self.record(event);
        }
    }
}
crate::definition::single_lane!(Transaction => Account);
