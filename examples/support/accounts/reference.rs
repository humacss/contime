//! Independent chronological oracle, compacted only behind retained inputs.
use crate::{
    accounts_example::model::{Account, Decision, Transaction},
    definition::Time,
};
use std::collections::BTreeMap;
pub struct Reference {
    anchor: Account,
    events: BTreeMap<(Time, u128), Transaction>,
}
impl Reference {
    pub fn new() -> Self {
        Self { anchor: Account { account_id: 1, ..Default::default() }, events: BTreeMap::new() }
    }
    pub fn insert(&mut self, event: Transaction) {
        self.events.entry((event.time, event.id)).or_insert(event);
    }
    /// Equal arrival timestamps still preserve transport handoff order.
    pub fn insert_arrival(&mut self, event: Transaction, order: u64) {
        self.events.insert((event.time, u128::from(order)), event);
    }
    pub fn at(&self, time: Time) -> Account {
        let mut account = self.anchor.clone();
        for event in self.events.values().take_while(|event| event.time <= time) {
            fold(&mut account, event);
        }
        account
    }
    pub fn compact(&mut self, before: Time) {
        let keys: Vec<_> = self.events.range(..(before, 0)).map(|(key, _)| *key).collect();
        for key in keys {
            fold(&mut self.anchor, &self.events.remove(&key).unwrap());
        }
    }
}
pub fn fold(account: &mut Account, event: &Transaction) {
    let next = account.balance + event.amount;
    let accepted = next >= 0;
    if accepted {
        account.balance = next;
    }
    account.events += 1;
    account.decisions.push(Decision { id: event.id, time: event.time, amount: event.amount, accepted, balance: account.balance });
    if account.decisions.len() > 64 {
        account.decisions.remove(0);
    }
}
