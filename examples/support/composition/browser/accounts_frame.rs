//! Adapt the account simulation to the existing budgeted browser scheduler.
use crate::{
    accounts::{Config, Driver},
    accounts_example::setup::AccountExample,
    support::monitored::{backend::Manual, Options},
};
use contime::contime_monitor::WorkerMonitor;
use std::time::Duration;
pub struct Frame {
    backend: Manual<AccountExample>,
    driver: Driver,
}
impl Frame {
    pub fn new(json: &str) -> Result<Self, String> {
        let values: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut args = vec!["--execution".into(), "manual".into()];
        for key in ["query_hz", "retention_ms", "checkpoint_interval", "duration"] {
            if let Some(value) = values.get(key).filter(|value| !value.is_null()) {
                args.extend([format!("--{}", key.replace('_', "-")), value.as_u64().ok_or("Invalid account setting")?.to_string()]);
            }
        }
        let config = Config::parse(args)?;
        let backend = Manual::new(&Options {
            snapshots: 1,
            workers: 1,
            retention_ms: config.retention_ms,
            checkpoint_interval: config.checkpoint_interval,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        Ok(Self { backend, driver: Driver::new(config) })
    }
}
impl crate::scheduler::Task for Frame {
    fn step(&mut self, now: Duration, stop: bool) -> Result<bool, String> {
        self.driver.step(&self.backend, &mut WorkerMonitor::attach(&self.backend), now, stop).map_err(|e| e.to_string())
    }
    fn report(&self) -> serde_json::Value {
        serde_json::json!({"elapsed":self.driver.now.as_secs_f64(),"accounts":self.driver.report()})
    }
}
