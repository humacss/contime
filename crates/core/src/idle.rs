//! Each caller subscribes to every component's activity channel. Until an
//! initial idle reply arrives, that component is not considered idle.
//! Components report working before dequeue and idle after processing and
//! outgoing sends finish. Registration is handled in their idle loops.
//!
//! The waiter keeps only the latest status per component. Once all are idle,
//! it inspects work queues and then checks for pending status messages. Any
//! pending message requires another receive iteration and a fresh queue check,
//! including a working/idle pair, so handoffs remain visible.
//! Queue checks use weak receiver references and cannot keep queues connected.
//!
//! Callers have independent subscriptions. Components do not wait for observer
//! acknowledgements; disconnected subscribers are removed on later reports.

use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Select};

use crate::{ConTime, Input};

/// Idle could not be confirmed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdleError {
    /// The caller's time budget expired while work remained.
    Timeout,
    /// A router or worker exited before idle could be confirmed.
    ComponentStopped,
}

impl std::fmt::Display for IdleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Timeout => "ConTime idle wait timed out",
            Self::ComponentStopped => "ConTime router or worker stopped before idle was confirmed",
        })
    }
}
impl std::error::Error for IdleError {}

impl<I: Input, S, W> ConTime<I, S, W> {
    /// Blocks the caller until work queues are empty and routers/workers idle.
    /// Stop submitting new work before calling to obtain a completion guarantee.
    /// Queued and active work are both observed, including router-to-worker
    /// handoffs. Does not advance time or cancel processing on timeout.
    /// Success observes idle; it does not prevent later submissions.
    ///
    /// # Errors
    /// Returns [`IdleError::Timeout`] when the time budget expires, or
    /// [`IdleError::ComponentStopped`] immediately when a component's activity
    /// channel disconnects. The deadline is checked on each receive iteration,
    /// even while status messages keep arriving.
    pub fn wait_until_idle(&self, timeout: Duration) -> Result<(), IdleError> {
        let started = Instant::now();
        let mut status_receivers = Vec::with_capacity(self.subscriptions.len());
        for subscription in &self.subscriptions {
            let (sender, receiver) = unbounded();
            subscription.send(sender).map_err(|_| IdleError::ComponentStopped)?;
            status_receivers.push(receiver);
        }
        // No initial report means not confirmed idle. Registration remains
        // queued while a component works; the idle loop replies when ready.
        let mut idle = vec![false; status_receivers.len()];
        loop {
            let remaining = timeout.checked_sub(started.elapsed()).ok_or(IdleError::Timeout)?;
            let (index, result) = {
                let mut ready = Select::new();
                for receiver in &status_receivers {
                    ready.recv(receiver);
                }
                // Working is reported before dequeue, idle after outgoing sends.
                // Check for a newer status AFTER inspecting all work queues.
                if idle.iter().all(|idle| *idle) && self.queues.iter().all(|empty| empty()) && ready.try_ready().is_err() {
                    return Ok(());
                }
                let selected = ready.select_timeout(remaining).map_err(|_| IdleError::Timeout)?;
                let index = selected.index();
                (index, selected.recv(&status_receivers[index]))
            };
            match result {
                Ok(working) => idle[index] = !working,
                Err(_) => return Err(IdleError::ComponentStopped),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoints::{ApplyBatch, ApplyEvents, Snapshot};
    use crate::types::testing::Time;

    #[derive(Clone, Default)]
    struct Value(Time);

    impl contime_checkpoints::Event for Value {
        type Time = Time;

        fn time(&self) -> Time {
            Time(0)
        }
    }
    impl Input for Value {
        fn event_id(&self) -> u128 {
            0
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(0);
        }
    }
    impl Snapshot for Value {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.0
        }
        fn set_time(&mut self, time: Time) {
            self.0 = time;
        }
    }
    impl ApplyEvents<Value> for Value {
        fn create(_: u128, _: &Value) -> Self {
            Self(Time(0))
        }
        fn apply_events(&mut self, _: ApplyBatch<'_, '_, Time, Value>) {}
    }

    fn core() -> ConTime<Value, Value, ()> {
        ConTime::start(
            crate::ConTimeConfig {
                router_count: 1,
                worker_count: 1,
                placement: contime_router::Placement::default(),
                pruning_interval: std::time::Duration::from_millis(100),

                history_retention: Time(0),
                worker: contime_worker::WorkerConfig {
                    maximum_dirty_age: Duration::from_millis(1),
                    replays_per_receive: 1,
                    deadline_compaction_minimum: 1_024,
                    deadline_compaction_multiplier: 2,
                },
                checkpoints: crate::checkpoints::CheckpointConfig { interval: 100 },
            },
            (),
        )
        .unwrap()
    }

    #[test]
    fn working_report_during_queue_check_invalidates_scan() {
        let mut core = core();
        let (subscribe, subscriptions) = unbounded::<crossbeam_channel::Sender<bool>>();
        let (published, status) = unbounded();
        let (release, finished) = unbounded::<()>();
        core.subscriptions = vec![subscribe];
        let component = std::thread::spawn(move || {
            let sender = subscriptions.recv().unwrap();
            sender.send(false).unwrap();
            published.send(sender.clone()).unwrap();
            let _ = finished.recv();
        });
        core.queues = vec![std::sync::Arc::new(move || {
            status.recv().unwrap().send(true).unwrap();
            true
        })];
        assert_eq!(core.wait_until_idle(Duration::from_millis(50)), Err(IdleError::Timeout));
        drop(release);
        component.join().unwrap();
        core.shutdown();
    }

    #[test]
    fn working_then_idle_requires_fresh_queue_inspection() {
        let mut core = core();
        let (subscribe, subscriptions) = unbounded::<crossbeam_channel::Sender<bool>>();
        let (published, status) = unbounded();
        let (release, finished) = unbounded::<()>();
        core.subscriptions = vec![subscribe];
        let component = std::thread::spawn(move || {
            let sender = subscriptions.recv().unwrap();
            sender.send(false).unwrap();
            published.send(sender.clone()).unwrap();
            let _ = finished.recv();
        });
        // A test-only counter drives a deterministic handoff during inspection.
        let once = std::sync::atomic::AtomicBool::new(false);
        core.queues = vec![std::sync::Arc::new(move || {
            if !once.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let tx = status.recv().unwrap();
                tx.send(true).unwrap();
                tx.send(false).unwrap();
                true
            } else {
                false
            }
        })];
        assert_eq!(core.wait_until_idle(Duration::from_millis(50)), Err(IdleError::Timeout));
        drop(release);
        component.join().unwrap();
        core.shutdown();
    }

    #[test]
    fn stopped_component_returns_without_waiting_for_the_deadline() {
        for registered in [false, true] {
            let mut core = core();
            let (subscribe, subscriptions) = unbounded::<crossbeam_channel::Sender<bool>>();
            core.subscriptions = vec![subscribe];
            let component = std::thread::spawn(move || {
                if registered {
                    let status = subscriptions.recv().unwrap();
                    status.send(true).unwrap();
                }
            });
            let (reply, result) = unbounded();
            let waiter = std::thread::spawn(move || {
                let outcome = core.wait_until_idle(Duration::from_secs(2));
                let _ = reply.send(outcome);
                core.shutdown();
            });
            let outcome = result.recv_timeout(Duration::from_millis(500));
            component.join().unwrap();
            waiter.join().unwrap();
            assert!(outcome.is_ok(), "component exit must not wait for the deadline");
            assert_eq!(outcome.unwrap(), Err(IdleError::ComponentStopped));
        }
    }

    #[test]
    fn disconnected_observer_cannot_confirm_idle() {
        let mut core = core();
        let (subscribe, subscriptions) = unbounded::<crossbeam_channel::Sender<bool>>();
        core.subscriptions = vec![subscribe];
        let (published, status) = unbounded();
        core.queues = vec![std::sync::Arc::new(move || {
            drop(status.recv().unwrap());
            true
        })];
        let component = std::thread::spawn(move || {
            let sender = subscriptions.recv().unwrap();
            sender.send(false).unwrap();
            published.send(sender).unwrap();
        });
        assert_eq!(core.wait_until_idle(Duration::from_millis(50)), Err(IdleError::ComponentStopped));
        component.join().unwrap();
        core.shutdown();
    }
}
