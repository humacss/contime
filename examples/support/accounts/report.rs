//! Bounded data-only interface shared with the Bevy renderer.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub id: u64,
    pub time_ms: u64,
    pub amount: i64,
    pub accepted: bool,
    pub balance: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub sequence: u64,
    pub transaction: u64,
    pub action_ms: u64,
    pub due_ms: u64,
    pub arrived_ms: Option<u64>,
    pub amount: i64,
    pub duplicate: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub clock_ms: u64,
    pub query_ms: u64,
    pub revision: u64,
    pub balance: i64,
    pub arrival_balance: i64,
    pub reconciled: Vec<Decision>,
    pub arrival_order: Vec<Decision>,
    pub deliveries: Vec<Delivery>,
    pub retries: u64,
    pub submitted: u64,
    pub done: bool,
}
impl Report {
    pub fn validate(&self) -> Result<(), String> {
        if self.query_ms > self.clock_ms
            || self.balance < 0
            || self.arrival_balance < 0
            || self.reconciled.len() > 64
            || self.arrival_order.len() > 64
            || self.deliveries.len() > 128
            || self.reconciled.iter().chain(&self.arrival_order).any(|d| d.balance < 0 || d.time_ms > self.query_ms)
            || self
                .deliveries
                .iter()
                .any(|d| d.due_ms < d.action_ms || d.arrived_ms.is_some_and(|at| at < d.action_ms || at > self.clock_ms))
        {
            return Err("Invalid account presentation report".into());
        }
        Ok(())
    }
}
