//! Caller-driven execution of a portable verification workload.
use crate::definition::Example;
use crate::verification::{Instance, Report, Workload};
use contime::manual::{FrameBudget, FrameReport, StopReason};
use std::time::Duration;

/// Host-independent verification scenario driven through the public manual instance.
pub struct Scenario<M: Example> {
    backend: Instance<M>,
    workload: Workload<M>,
}
impl<M: Example> Scenario<M> {
    pub fn new(batch_size: usize) -> Result<Self, String> {
        let (backend, workload) = Workload::<M>::new(batch_size)?;
        Ok(Self { backend, workload })
    }
    pub fn inject_mismatch(&mut self) {
        self.workload.inject_mismatch();
    }
    pub fn enqueue_round(&mut self) -> Result<(), String> {
        self.workload.enqueue_round(&mut self.backend)
    }
    /// Make one processing step; this path needs no clock or browser host.
    pub fn process(&mut self) -> Result<Report, String> {
        if let Some(error) = &self.workload.failed {
            return Err(error.clone());
        }
        let progressed = self.backend.process().map_err(|error| error.to_string())?;
        let report = FrameReport {
            steps: usize::from(progressed),
            elapsed: Duration::ZERO,
            overrun: false,
            queued: self.backend.queued(),
            stopped: if progressed { StopReason::BudgetExhausted } else { StopReason::NoReadyWork },
        };
        self.workload.collect(&mut self.backend, report)
    }
    /// Process within a host-supplied allowance and monotonic clock.
    pub fn process_with_clock(&mut self, budget: FrameBudget, clock: impl FnMut() -> Duration) -> Result<Report, String> {
        if let Some(error) = &self.workload.failed {
            return Err(error.clone());
        }
        let report = self.backend.process_with_clock(budget, clock).map_err(|e| e.to_string())?;
        self.workload.collect(&mut self.backend, report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CounterExample, HashExample};
    #[test]
    fn reversed_delivery_and_retention_verify_with_one_step_per_frame() {
        verify_rounds::<CounterExample>();
        verify_rounds::<HashExample>();
    }
    fn verify_rounds<M: Example>() {
        let mut demo = Scenario::<M>::new(32).unwrap();
        for _ in 0..8 {
            demo.enqueue_round().unwrap();
            let mut frames = 0;
            loop {
                let report = demo.process().unwrap();
                assert!(report.steps <= 1);
                frames += 1;
                assert!(frames < 1000);
                if !report.busy {
                    break;
                }
            }
            assert!(frames > 32);
        }
        assert_eq!(demo.workload.verified, 24);
        assert_eq!(demo.workload.duplicates, 8);
        assert_eq!(demo.workload.rejected, 7);
        assert_eq!(M::hash(&demo.workload.queried), M::hash(&demo.workload.produced));
        assert_eq!(M::value(&demo.workload.queried), M::value(&demo.workload.produced));
        assert_eq!(M::count(&demo.workload.queried), 256);
    }
    #[test]
    fn mismatch_is_fatal_and_zero_budget_leaves_work_pending() {
        verify_failure::<CounterExample>();
        verify_failure::<HashExample>();
    }
    fn verify_failure<M: Example>() {
        let mut demo = Scenario::<M>::new(8).unwrap();
        demo.enqueue_round().unwrap();
        let report = demo.process_with_clock(FrameBudget { duration: Duration::ZERO, max_steps: 1000 }, || Duration::ZERO).unwrap();
        assert_eq!(report.steps, 0);
        assert!(report.pending > 0);
        demo.inject_mismatch();
        assert!(demo
            .process_with_clock(FrameBudget { duration: Duration::from_millis(1), max_steps: 1000 }, || Duration::ZERO)
            .unwrap_err()
            .contains("mismatch"));
        assert!(demo.process().is_err());
        assert!(demo.enqueue_round().is_err());
    }
    #[test]
    fn budgeted_clock_yields_and_finishes_both_domains() {
        verify_clock::<CounterExample>();
        verify_clock::<HashExample>();
    }
    fn verify_clock<M: Example>() {
        let mut demo = Scenario::<M>::new(32).unwrap();
        demo.enqueue_round().unwrap();
        for frame in 0..1000 {
            let mut tick = 0;
            let report = demo
                .process_with_clock(FrameBudget { duration: Duration::from_millis(2), max_steps: 3 }, || {
                    tick += 1;
                    Duration::from_millis(tick)
                })
                .unwrap();
            assert!(report.steps <= 3);
            if !report.busy {
                assert!(frame > 0);
                assert_eq!(report.verified, 3);
                return;
            }
        }
        panic!("budgeted processing failed to finish");
    }
}
