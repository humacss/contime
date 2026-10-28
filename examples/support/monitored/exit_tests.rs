//! Subprocess probes keep real backend work while detecting premature teardown.
use super::backend::Backend;
use crate::definition::{Example, Time};
use contime_monitor::{WorkerMonitorError, WorkerStatus, WorkerStatusSource};
use crossbeam_channel::{Receiver, Sender};
use std::{path::PathBuf, process::Command};
type Failure = Box<dyn std::error::Error>;

struct Teardown(PathBuf);
impl Drop for Teardown {
    fn drop(&mut self) {
        std::fs::write(&self.0, "backend torn down").unwrap();
    }
}
/// Delegate every operation to a real backend, marking when ownership is released.
pub struct Observed<B> {
    inner: B,
    _teardown: Teardown,
}
impl<B> Observed<B> {
    pub fn new(inner: B, marker: PathBuf) -> Self {
        Self { inner, _teardown: Teardown(marker) }
    }
}
impl<B: WorkerStatusSource<Time>> WorkerStatusSource<Time> for Observed<B> {
    fn worker_count(&self) -> usize {
        self.inner.worker_count()
    }
    fn request_worker_status(&self) -> Result<Receiver<WorkerStatus<Time>>, WorkerMonitorError> {
        self.inner.request_worker_status()
    }
}
impl<M: Example, B: Backend<M>> Backend<M> for Observed<B> {
    fn submit(&self, events: Vec<M::Event>) -> Result<(), Failure> {
        self.inner.submit(events)
    }
    fn advance(&self, time: Time) -> Result<(), Failure> {
        self.inner.advance(time)
    }
    fn query(&self, time: Time, count: usize, response: Sender<Vec<Box<M::State>>>) -> Result<(), Failure> {
        self.inner.query(time, count, response)
    }
    fn check_rejections(&self) -> Result<(), Failure> {
        self.inner.check_rejections()
    }
    fn process(&self) -> Result<bool, Failure> {
        self.inner.process()
    }
    fn can_submit(&self) -> bool {
        self.inner.can_submit()
    }
    fn shutdown(self) -> Result<(), Failure> {
        self.inner.shutdown()
    }
}

pub fn assert_fast_exit(test_name: &str, run: impl FnOnce(PathBuf)) {
    const CHILD: &str = "CONTIME_BACKEND_EXIT_TEST";
    if let Some(marker) = std::env::var_os(CHILD) {
        run(marker.into());
        panic!("successful standalone run returned instead of exiting");
    }
    let marker = marker(test_name);
    let _ = std::fs::remove_file(&marker);
    let output =
        Command::new(std::env::current_exe().unwrap()).args(["--exact", test_name, "--nocapture"]).env(CHILD, &marker).output().unwrap();
    let torn_down = marker.exists();
    let _ = std::fs::remove_file(&marker);
    assert!(!torn_down, "standalone run destroyed its backend before exit: {output:?}");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Validated") || stdout.contains("DRAINED (validated)"), "missing validated summary: {stdout}");
}
pub fn marker(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("contime-teardown-{}-{}.txt", std::process::id(), name.replace("::", "-")))
}
