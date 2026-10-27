#![cfg(feature = "workers")]
use contime_monitor::{WorkerMonitor, WorkerMonitorError, WorkerStatusSource};
use contime_worker::WorkerStatus;
use crossbeam_channel::{Receiver, Sender};
use std::{cell::RefCell, num::NonZeroUsize, sync::Arc, time::Duration};

struct Source {
    count: usize,
    sender: RefCell<Option<Sender<WorkerStatus<u64>>>>,
}
impl Source {
    fn new(count: usize) -> Self {
        Self { count, sender: RefCell::new(None) }
    }
    fn send(&self, id: usize) {
        self.sender.borrow().as_ref().unwrap().send(status(id)).unwrap();
    }
    fn close(&self) {
        self.sender.borrow_mut().take();
    }
}
impl WorkerStatusSource<u64> for Source {
    fn worker_count(&self) -> usize {
        self.count
    }
    fn request_worker_status(&self) -> Result<Receiver<WorkerStatus<u64>>, WorkerMonitorError> {
        let (sender, receiver) = crossbeam_channel::unbounded();
        *self.sender.borrow_mut() = Some(sender);
        Ok(receiver)
    }
}
fn status(worker_id: usize) -> WorkerStatus<u64> {
    WorkerStatus {
        worker_id,
        history_stores: 3,
        metadata_only: 2,
        pending_histories: 1,
        due_histories: 1,
        processing_target: 7,
        admission_horizon: 2,
        pending_prune_passes: 0,
        awaiting_resolution: None,
        counters: None,
    }
}
#[test]
fn only_complete_closed_response_sets_are_published_sorted() {
    let source = Source::new(2);
    let mut monitor = WorkerMonitor::attach(&source);
    monitor.begin(Duration::ZERO).unwrap();
    assert!(matches!(monitor.begin(Duration::ZERO), Err(WorkerMonitorError::Busy)));
    source.send(1);
    assert!(monitor.poll(Duration::from_millis(1)).unwrap().is_none());
    source.send(0);
    assert!(monitor.poll(Duration::from_millis(2)).unwrap().is_none());
    source.close();
    let reading = monitor.poll(Duration::from_millis(3)).unwrap().unwrap();
    assert_eq!(reading.started_at, Duration::ZERO);
    assert_eq!(reading.completed_at, Duration::from_millis(3));
    assert_eq!(reading.workers.iter().map(|s| s.worker_id).collect::<Vec<_>>(), [0, 1]);
    assert!(Arc::ptr_eq(&reading, monitor.latest().unwrap()));
}
#[test]
fn malformed_responses_never_publish() {
    for (count, ids, expected) in [
        (2, vec![0], WorkerMonitorError::Incomplete { expected: 2, received: 1 }),
        (2, vec![0, 0], WorkerMonitorError::DuplicateWorker(0)),
        (2, vec![2], WorkerMonitorError::UnknownWorker(2)),
    ] {
        let source = Source::new(count);
        let mut monitor = WorkerMonitor::attach(&source);
        monitor.begin(Duration::ZERO).unwrap();
        for id in ids {
            source.send(id);
        }
        source.close();
        assert_eq!(monitor.poll(Duration::ZERO).unwrap_err(), expected);
        assert!(monitor.latest().is_none());
        assert!(monitor.history().next().is_none());
        monitor.begin(Duration::ZERO).unwrap();
    }
}
#[test]
fn backwards_clocks_do_not_discard_valid_pending_reports() {
    let source = Source::new(1);
    let mut monitor = WorkerMonitor::attach(&source);
    monitor.begin(Duration::from_secs(2)).unwrap();
    source.send(0);
    source.close();
    assert_eq!(monitor.poll(Duration::from_secs(1)).unwrap_err(), WorkerMonitorError::ClockWentBackwards);
    assert!(monitor.poll(Duration::from_secs(3)).unwrap().is_some());
    assert_eq!(monitor.begin(Duration::from_secs(2)).unwrap_err(), WorkerMonitorError::ClockWentBackwards);
}
#[test]
fn cancellation_disconnects_transport_and_retains_previous_reading() {
    let source = Source::new(1);
    let mut monitor = WorkerMonitor::attach(&source);
    monitor.begin(Duration::ZERO).unwrap();
    source.send(0);
    source.close();
    monitor.poll(Duration::ZERO).unwrap().unwrap();
    monitor.begin(Duration::ZERO).unwrap();
    monitor.cancel();
    assert!(source.sender.borrow().as_ref().unwrap().send(status(0)).is_err());
    assert!(monitor.latest().is_some());
    assert_eq!(monitor.poll(Duration::ZERO).unwrap_err(), WorkerMonitorError::NoActiveRequest);
}
#[test]
fn history_is_bounded_and_readings_can_outlive_it() {
    let source = Source::new(1);
    let mut monitor = WorkerMonitor::with_capacity(&source, NonZeroUsize::new(2).unwrap());
    let mut held = None;
    for i in 0..4 {
        let at = Duration::from_secs(i);
        monitor.begin(at).unwrap();
        source.send(0);
        source.close();
        let reading = monitor.poll(at).unwrap().unwrap();
        if i == 0 {
            held = Some(reading);
        }
    }
    assert_eq!(monitor.history().count(), 2);
    assert_eq!(monitor.history().next().unwrap().started_at, Duration::from_secs(2));
    assert_eq!(held.unwrap().started_at, Duration::ZERO);
}
#[test]
fn empty_sources_are_rejected_without_sending() {
    let source = Source::new(0);
    let mut monitor = WorkerMonitor::attach(&source);
    assert_eq!(monitor.begin(Duration::ZERO).unwrap_err(), WorkerMonitorError::EmptySource);
    assert!(source.sender.borrow().is_none());
}

#[test]
fn failed_request_does_not_start_collection_or_advance_clock() {
    struct Failed;
    impl WorkerStatusSource<u64> for Failed {
        fn worker_count(&self) -> usize {
            1
        }
        fn request_worker_status(&self) -> Result<Receiver<WorkerStatus<u64>>, WorkerMonitorError> {
            Err(WorkerMonitorError::Source("closed".into()))
        }
    }
    let mut monitor = WorkerMonitor::attach(&Failed);
    assert_eq!(monitor.begin(Duration::from_secs(10)).unwrap_err(), WorkerMonitorError::Source("closed".into()));
    assert_eq!(monitor.poll(Duration::ZERO).unwrap_err(), WorkerMonitorError::NoActiveRequest);
}
#[test]
fn malformed_new_collection_keeps_previous_complete_reading() {
    let source = Source::new(1);
    let mut monitor = WorkerMonitor::attach(&source);
    monitor.begin(Duration::ZERO).unwrap();
    source.send(0);
    source.close();
    let first = monitor.poll(Duration::ZERO).unwrap().unwrap();
    monitor.begin(Duration::from_secs(1)).unwrap();
    source.close();
    assert!(matches!(monitor.poll(Duration::from_secs(2)), Err(WorkerMonitorError::Incomplete { .. })));
    assert!(Arc::ptr_eq(&first, monitor.latest().unwrap()));
    assert_eq!(monitor.history().count(), 1);
}
