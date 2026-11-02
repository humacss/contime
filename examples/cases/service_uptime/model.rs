//! Timestamped health changes. Time is application nanoseconds, never arrival time.
use crate::definition::Time;
use contime::prelude::*;
const HISTORY: usize = 256;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Healthy,
    Degraded,
    Down,
}
#[derive(Clone, Debug, ContimeEvent)]
pub struct StatusChanged {
    #[contime(id)]
    pub id: u128,
    pub service_id: u128,
    #[contime(time)]
    pub time: Time,
    pub status: Status,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interval {
    pub start: Time,
    pub end: Time,
    pub status: Status,
}
#[derive(Clone, Debug, Default, ContimeSnapshot)]
#[contime(events = [StatusChanged])]
pub struct Service {
    #[contime(id)]
    pub service_id: u128,
    #[contime(time)]
    pub time: Time,
    pub status: Status,
    /// Integration anchor is separate from snapshot metadata modified by queries/pruning.
    pub changed_at: Time,
    pub uptime: u64,
    pub downtime: u64,
    pub degraded: u64,
    pub events: u64,
    pub intervals: Vec<Interval>,
}
impl SnapshotApply for Service {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<StatusChanged>() {
            self.apply_change(event);
        }
    }
}
impl Service {
    pub fn apply_change(&mut self, event: &StatusChanged) {
        // The first input establishes the service lifetime (seeds use the one-nanosecond origin).
        let duration = if self.events == 0 { 0 } else { event.time.0.saturating_sub(self.changed_at.0) };
        if self.status == Status::Down {
            self.downtime += duration;
        } else {
            self.uptime += duration;
        }
        if self.status == Status::Degraded {
            self.degraded += duration;
        }
        if duration > 0 {
            self.intervals.push(Interval { start: self.changed_at, end: event.time, status: self.status });
            if self.intervals.len() > HISTORY {
                self.intervals.remove(0);
            }
        }
        self.changed_at = event.time;
        self.status = event.status;
        self.events += 1;
    }
    pub fn totals_at(&self, time: Time) -> (u64, u64, u64) {
        let duration = time.0.saturating_sub(self.changed_at.0);
        (
            self.uptime + if self.status != Status::Down { duration } else { 0 },
            self.downtime + if self.status == Status::Down { duration } else { 0 },
            self.degraded + if self.status == Status::Degraded { duration } else { 0 },
        )
    }
}
crate::definition::single_lane!(StatusChanged => Service);
