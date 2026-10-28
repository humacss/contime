//! Adapt the shared movement driver to the browser's caller-driven frame loop.
use crate::{
    movement::{Config, Driver},
    movement_example::setup::MovementExample,
    support::monitored::{backend::Manual, Options},
};
use contime::contime_monitor::WorkerMonitor;
use std::time::Duration;
pub struct Frame {
    backend: Manual<MovementExample>,
    driver: Driver,
}
impl Frame {
    pub fn new(config: Config) -> Result<Self, String> {
        let options = Options {
            snapshots: config.clients,
            workers: 1,
            retention_ms: config.retention_ms,
            checkpoint_interval: config.checkpoint_interval,
            headless: false,
            ..Default::default()
        };
        let backend = Manual::new(&options).map_err(|e| e.to_string())?;
        let driver = Driver::new(&backend, config).map_err(|e| e.to_string())?;
        Ok(Self { backend, driver })
    }
    pub fn input(&mut self, direction: &str) -> Result<(), String> {
        self.driver.input(crate::movement::direction(direction)?)
    }
}
impl crate::scheduler::Task for Frame {
    fn step(&mut self, now: Duration, stop: bool) -> Result<bool, String> {
        // Manual status is captured synchronously; collection never remains pending.
        let mut monitor = WorkerMonitor::attach(&self.backend);
        self.driver.step(&self.backend, &mut monitor, now, stop).map_err(|e| e.to_string())
    }
    fn report(&self) -> serde_json::Value {
        serde_json::json!({"elapsed": self.driver.now.as_secs_f64(), "submitted": self.driver.submitted,
            "applications": self.driver.submitted, "produced": self.driver.submitted, "completed": self.driver.completed,
            "outstanding": self.driver.submitted-self.driver.completed, "queued": self.backend.queued(), "queries": self.driver.updates,
            "target_rate": self.driver.config.clients.saturating_sub(1) as u64*self.driver.config.input_hz, "query_ms":0.0,
            "workers_status": crate::worker_report::workers(self.driver.baseline.as_deref(), self.driver.latest.as_deref()),
            "movement": self.driver.report()})
    }
}
