//! The account lane plugs into the same environment adapters as the other cases.
use super::model::{Account, Transaction};
use crate::definition::{Example, Time};
pub struct AccountExample;
impl Example for AccountExample {
    type Event = Transaction;
    type State = Account;
    const TITLE: &'static str = "Account balances";
    const VALUE_LABEL: &'static str = "Credits";
    fn event(id: u128, time: Time, targets: Vec<u128>, amount: i64) -> Transaction {
        Transaction { id, time, account_id: targets.first().copied().unwrap_or(1), amount }
    }
    fn id(state: &Account) -> u128 {
        state.account_id
    }
    fn count(state: &Account) -> u64 {
        state.events
    }
    fn value(state: &Account) -> i64 {
        state.balance
    }
    fn record(state: &mut Account, event: &Transaction) {
        state.record(event);
    }
}
