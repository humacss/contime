//! Case-agnostic ConTime startup and caller-driven execution resources.
//! No workload generation, monitoring, validation, or presentation policy lives here.
#![allow(dead_code)] // Each host compiles only the execution paths it uses.
use crate::definition::{Example, Time};
use crossbeam_channel::Sender;
type Failure = Box<dyn std::error::Error>;
pub struct Config {
    pub workers: usize,
    pub retention_ms: u64,
    pub checkpoint_interval: u64,
}
#[cfg(feature = "runtime")]
pub type Runtime<M> = contime::RawConTime<<M as Example>::Event, <M as Example>::State, ()>;

#[cfg(feature = "runtime")]
pub fn runtime<M: Example>(config: &Config) -> Result<Runtime<M>, Failure>
where
    M::Event: Send + Sync,
    M::State: Send,
{
    use contime::{checkpoints::CheckpointConfig, ConTimeConfig, Placement};
    use std::time::Duration;
    Ok(contime::RawConTime::start(
        ConTimeConfig {
            router_count: 1,
            worker_count: config.workers,
            placement: Placement::default(),
            history_retention: Time(config.retention_ms * 1_000_000),
            pruning_interval: Duration::from_millis(100),
            checkpoints: CheckpointConfig { interval: config.checkpoint_interval },
        },
        (),
    )?)
}

#[cfg(feature = "manual")]
mod manual {
    use super::*;
    use contime::{
        checkpoints::CheckpointConfig,
        manual::{ManualConfig, RawManualConTime},
    };
    use std::{
        cell::{Cell, RefCell},
        num::NonZeroUsize,
    };
    const QUEUE_CAPACITY: usize = 32;
    // Three batch/advance/prune messages plus a validation query fit after admission.
    const RESERVED: usize = 4;
    pub struct Manual<M: Example> {
        contime: RefCell<RawManualConTime<M::Event, M::State>>,
        retention: Time,
        last_prune: Cell<Time>,
    }
    impl<M: Example> Manual<M> {
        pub fn inspect<R>(&self, read: impl FnOnce(&RawManualConTime<M::Event, M::State>) -> R) -> R {
            read(&self.contime.borrow())
        }
        pub fn queued(&self) -> usize {
            self.contime.borrow().queued()
        }
        pub fn new(config: &Config) -> Result<Self, Failure> {
            if config.workers != 1 {
                return Err("Manual execution uses one worker".into());
            }
            Ok(Self {
                contime: RefCell::new(RawManualConTime::new(
                    ManualConfig {
                        queue_capacity: NonZeroUsize::new(QUEUE_CAPACITY).unwrap(),
                        checkpoints: CheckpointConfig { interval: config.checkpoint_interval },
                    },
                    (),
                )),
                retention: Time(config.retention_ms * 1_000_000),
                last_prune: Cell::new(Time(0)),
            })
        }
    }
    impl<M: Example> Manual<M> {
        pub fn submit(&self, events: Vec<M::Event>) -> Result<(), Failure> {
            self.contime.borrow_mut().apply(events)?;
            Ok(())
        }
        pub fn advance(&self, time: Time) -> Result<(), Failure> {
            let mut contime = self.contime.borrow_mut();
            contime.advance_to(time)?;
            // Manual owns retention explicitly. Prune through its existing barrier,
            // after advancement, at the same 100 ms interval as the Runtime demo.
            if time.0.saturating_sub(self.last_prune.get().0) >= 100_000_000 {
                contime.prune_before(Time(time.0.saturating_sub(self.retention.0)))?;
                self.last_prune.set(time);
            }
            Ok(())
        }
        pub fn query(&self, time: Time, count: usize, response: Sender<Vec<Box<M::State>>>) -> Result<(), Failure> {
            self.contime.borrow_mut().query_at(time, 1..=count as u128, move |states| {
                let _ = response.send(states);
            })?;
            Ok(())
        }
        pub fn check_rejections(&self) -> Result<(), Failure> {
            let rejected = self.contime.borrow_mut().take_rejections();
            if !rejected.is_empty() {
                return Err(format!("Rejected events: {rejected:?}").into());
            }
            Ok(())
        }
        pub fn process(&self) -> Result<bool, Failure> {
            Ok(self.contime.borrow_mut().process()?)
        }
        pub fn can_submit(&self) -> bool {
            self.contime.borrow().queued() <= QUEUE_CAPACITY - RESERVED
        }
        pub fn shutdown(self) -> Result<(), Failure> {
            while self.process()? {}
            self.check_rejections()
        }
    }
}
#[cfg(feature = "manual")]
pub use manual::Manual;
