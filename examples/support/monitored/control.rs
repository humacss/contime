//! Example-only closed-loop load control. Monitor readings are the only feedback.
use super::Time;
use contime_monitor::WorkerReading;
use std::time::Duration;
const WINDOW: Duration = Duration::from_millis(50);
// Automatic memory bound in target applications, independent of query cadence.
const MAX_APPLICATIONS: u64 = 1_000_000;
struct Window {
    started: Duration,
    quota: u64,
    issued: u64,
    target: Time,
    sealed_at: Option<Duration>,
}
pub struct Controller {
    rate: u64,
    fanout: u64,
    source_limit: u64,
    minimum_group: u64,
    maximum: bool,
    stopped: bool,
    produced: u64,
    completed: u64,
    applications: u64,
    window: Option<Window>,
}
impl Controller {
    pub fn new(initial_rate: u64, fanout: usize, maximum: bool) -> Self {
        Self {
            rate: initial_rate,
            fanout: fanout as u64,
            source_limit: MAX_APPLICATIONS / fanout as u64,
            minimum_group: 1,
            maximum,
            stopped: false,
            produced: 0,
            completed: 0,
            applications: 0,
            window: None,
        }
    }
    pub fn limit_sources(&mut self, limit: u64) {
        self.source_limit = self.source_limit.min(limit.max(1));
    }
    pub fn reverse_groups(&mut self) {
        self.minimum_group = 2;
    }
    pub fn minimum_submission(&self) -> u64 {
        self.window.as_ref().map_or(1, |window| self.minimum_group.min(window.quota - window.issued))
    }
    pub fn allowance(&mut self, now: Duration) -> u64 {
        if self.stopped {
            return 0;
        }
        let capacity = self.source_limit;
        let window = self.window.get_or_insert_with(|| Window {
            started: now,
            quota: (self.rate.div_ceil(20)).max(self.minimum_group).clamp(1, capacity),
            issued: 0,
            target: Time(0),
            sealed_at: None,
        });
        if window.sealed_at.is_some() {
            return 0;
        }
        let credit = if self.maximum {
            u64::MAX
        } else {
            let due = (now.as_nanos() * u128::from(self.rate) / 1_000_000_000).min(u128::from(u64::MAX)) as u64;
            due.saturating_sub(self.produced)
        };
        (window.quota - window.issued).min(1_024).min(credit)
    }
    pub fn submitted(&mut self, count: u64, applications: u64, time: Time, now: Duration) -> Result<(), String> {
        let window = self.window.as_mut().ok_or("No production window")?;
        if count == 0 || window.sealed_at.is_some() || count > window.quota - window.issued {
            return Err("Submission exceeds current production window".into());
        }
        if applications < count || applications > count * self.fanout {
            return Err("Invalid target application count".into());
        }
        self.applications = self.applications.checked_add(applications).ok_or("Target application count overflow")?;
        self.produced = self.produced.checked_add(count).ok_or("Source event count overflow")?;
        window.issued += count;
        window.target = time;
        if window.issued == window.quota {
            window.sealed_at = Some(now);
        }
        Ok(())
    }
    pub fn stop(&mut self, now: Duration) {
        self.stopped = true;
        if let Some(window) = &mut self.window {
            if window.issued == 0 {
                self.window = None;
            } else {
                window.sealed_at.get_or_insert(now);
            }
        }
    }
    /// With only this host producing unique events, exact insertion accounting
    /// plus empty worker schedules through the window target proves completion.
    /// Raw replay traversal totals are intentionally never inspected here.
    pub fn observe(&mut self, reading: &WorkerReading<Time>) -> Result<bool, String> {
        let Some(window) = &self.window else { return Ok(false) };
        let Some(sealed) = window.sealed_at else { return Ok(false) };
        if reading.started_at < sealed {
            return Ok(false);
        }
        if reading.workers.is_empty() {
            return Err("Empty worker reading".into());
        }
        let mut inserted = 0_u128;
        for worker in &reading.workers {
            let counts = worker.counters.ok_or("Worker counters are required for Maximum control")?;
            if counts.saturated {
                return Err("Saturated worker counts cannot establish completion".into());
            }
            inserted += u128::from(counts.inserted_applications);
        }
        let expected = u128::from(self.applications);
        if inserted > expected {
            return Err("Unexpected insertions: this controller requires an exclusive producer".into());
        }
        if inserted != expected
            || reading.workers.iter().any(|worker| worker.pending_histories != 0 || worker.processing_target < window.target)
        {
            return Ok(false);
        }
        let elapsed = reading
            .completed_at
            .checked_sub(window.started)
            .filter(|d| !d.is_zero())
            .ok_or("Window completion needs a positive elapsed observation interval")?;
        if self.maximum && !self.stopped {
            let observed = (u128::from(window.issued) * 1_000_000_000 / elapsed.as_nanos()).min(u128::from(u64::MAX)) as u64;
            self.rate = if elapsed <= WINDOW {
                self.rate.saturating_add((self.rate / 5).max(1)).max(observed.saturating_add(observed / 10))
            } else {
                observed.saturating_sub((observed / 10).max(1))
            };
            self.rate = self.rate.clamp(1, (MAX_APPLICATIONS / self.fanout) * 20);
        }
        self.completed = self.produced;
        self.window = None;
        Ok(true)
    }
    pub fn rate(&self) -> u64 {
        self.rate
    }
    pub fn produced(&self) -> u64 {
        self.produced
    }
    pub fn completed(&self) -> u64 {
        self.completed
    }
    pub fn outstanding(&self) -> u64 {
        self.produced - self.completed
    }
    pub fn waiting(&self) -> bool {
        self.window.as_ref().is_some_and(|w| w.sealed_at.is_some())
    }
    pub fn window_started(&self) -> Option<Duration> {
        self.window.as_ref().map(|w| w.started)
    }
}
#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
