//! Adapt the shared uptime driver to the browser's caller-driven frame loop.
use crate::{
    support::monitored::{backend::Manual, Options},
    uptime::{Config, Driver},
    uptime_example::setup::ServiceExample,
};
use contime::contime_monitor::WorkerMonitor;
use std::time::Duration;
pub struct Frame {
    backend: Manual<ServiceExample>,
    driver: Driver,
}
impl Frame {
    pub fn new(config: Config) -> Result<Self, String> {
        let options = Options {
            snapshots: config.services,
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
            "target_rate": self.driver.config.services as u64*1000/self.driver.config.cadence_ms, "query_ms":0.0,
            "workers_status": crate::worker_report::workers(self.driver.baseline.as_deref(), self.driver.latest.as_deref()),
            "uptime": self.driver.report()})
    }
}
