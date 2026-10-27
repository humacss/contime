//! Frame scheduling for an injected task. No cases or workload policy live here.
use std::time::Duration;
pub trait Task {
    fn step(&mut self, now: Duration, stop: bool) -> Result<bool, String>;
    fn report(&self) -> serde_json::Value;
}
pub struct Session<T> {
    task: T,
    stopped: bool,
    done: bool,
}
impl<T: Task> Session<T> {
    pub fn new(task: T) -> Self {
        Self { task, stopped: false, done: false }
    }
    pub fn task_mut(&mut self) -> &mut T {
        &mut self.task
    }
    /// The allowance includes generation, queries, pruning and processing. A
    /// complete shared step may overrun it; clock checks occur between steps.
    pub fn frame(
        &mut self,
        elapsed_ms: f64,
        milliseconds: f64,
        max_steps: usize,
        stop: bool,
        mut clock: impl FnMut() -> f64,
    ) -> Result<String, String> {
        if !elapsed_ms.is_finite()
            || elapsed_ms < 0.0
            || !milliseconds.is_finite()
            || !(0.01..=1000.0).contains(&milliseconds)
            || !(1..=100_000).contains(&max_steps)
        {
            return Err("Invalid elapsed time or processing allowance".into());
        }
        self.stopped |= stop;
        let start = clock();
        let mut steps = 0;
        while !self.done && steps < max_steps && clock() - start < milliseconds {
            let spent = (clock() - start).max(0.0);
            let active = Duration::try_from_secs_f64((elapsed_ms + spent) / 1000.0).map_err(|_| "Invalid active elapsed time")?;
            self.done = self.task.step(active, self.stopped)?;
            steps += 1;
        }
        let frame_ms = (clock() - start).max(0.0);
        let mut report = self.task.report();
        report["done"] = self.done.into();
        report["steps"] = steps.into();
        report["frame_ms"] = frame_ms.into();
        report["overrun"] = (frame_ms > milliseconds).into();
        Ok(report.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Work {
        calls: u64,
        stopping_calls: u64,
    }
    impl Task for Work {
        fn step(&mut self, _now: Duration, stop: bool) -> Result<bool, String> {
            self.calls += 1;
            self.stopping_calls += u64::from(stop);
            Ok(self.stopping_calls == 2)
        }
        fn report(&self) -> serde_json::Value {
            serde_json::json!({"calls": self.calls})
        }
    }
    fn report(session: &mut Session<Work>, steps: usize, stop: bool) -> serde_json::Value {
        serde_json::from_str(&session.frame(0.0, 1.0, steps, stop, || 0.0).unwrap()).unwrap()
    }
    #[test]
    fn injected_work_obeys_step_limits_and_latched_stop_and_is_not_called_after_completion() {
        let mut session = Session::new(Work::default());
        assert_eq!(report(&mut session, 2, false)["calls"], 2);
        assert_eq!(report(&mut session, 1, true)["done"], false);
        let final_report = report(&mut session, 8, false);
        assert_eq!(final_report["calls"], 4);
        assert_eq!(final_report["steps"], 1);
        assert_eq!(final_report["done"], true);
        let completed = report(&mut session, 8, false);
        assert_eq!(completed["calls"], 4);
        assert_eq!(completed["steps"], 0);
    }
    #[test]
    fn exhausted_clock_allowance_never_calls_injected_work() {
        let mut session = Session::new(Work::default());
        let mut clock = 0.0;
        let result = session
            .frame(0.0, 0.5, 10, false, || {
                clock += 1.0;
                clock
            })
            .unwrap();
        let report: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(report["calls"], 0);
        assert_eq!(report["steps"], 0);
    }
}
