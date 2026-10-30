//! Two counters driven by a host loop, without ConTime background threads.
//!
//! Included as a complete code sample in the Manual Rust documentation.
use contime::prelude::*;
use std::{
    cell::Cell,
    num::NonZeroUsize,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);
impl Timestamp for Time {
    fn minimum() -> Self {
        Self(0)
    }
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool {
        true
    }
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}

struct Count {
    id: u128,
    time: Time,
    counters: Vec<u128>,
}
impl :: contime :: checkpoints :: Event for Count { type Time = Time ; fn time (& self) -> Self :: Time { self . time } }

impl EventId for Count {
    fn event_id(&self) -> u128 {
        self.id
    }
}

#[derive(Clone, Default)]
struct Counter {
    id: u128,
    time: Time,
    value: u64,
}
impl :: contime :: checkpoints :: Snapshot for Counter { type Time = Time ; fn time (& self) -> & Self :: Time { & self . time } fn set_time (& mut self , time : Self :: Time) { self . time = time ; } }


impl SnapshotId for Counter {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}
impl SnapshotApply for Counter {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Count>(|event, emit| {
            for id in &event.counters {
                emit(*id);
            }
        });
    }
    fn create(snapshot_id: u128, _event: contime::EventRef<'_>) -> Self {
        Self { id: snapshot_id, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        self.value += batch.events::<Count>().count() as u64;
    }
}

#[derive(ContimeModule)]
#[contime_module(time = Time, events = [Count], snapshots = [Counter])]
struct Counters;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut contime = ManualConTime::<Counters>::new(ManualConfig {
        queue_capacity: NonZeroUsize::new(128).unwrap(),
        checkpoints: CheckpointConfig { interval: 100 },
    });
    let start = Instant::now();
    let mut next_frame = start;
    let mut submitted = 0;
    let rendered = Rc::new(Cell::new(0));
    while rendered.get() < 40 {
        // One host task submits events and requests a display update at 20 Hz.
        if submitted < 40 && Instant::now() >= next_frame {
            submitted += 1;
            let frame = submitted;
            let time = Time(start.elapsed().as_millis() as u64);
            contime.apply([Count { id: frame, time, counters: if frame % 2 == 0 { vec![1, 2] } else { vec![1] } }.into()])?;
            contime.advance_to(time)?;
            let rendered = rendered.clone();
            contime.query_at(time, [1, 2], move |states| {
                let mut values = states
                    .into_iter()
                    .map(|state| {
                        let CountersSnapshotLanes::Counter(state) = *state;
                        (state.id, state.value)
                    })
                    .collect::<Vec<_>>();
                values.sort_unstable_by_key(|(id, _)| *id);
                println!("Frame {frame:2}: {values:?}");
                rendered.set(frame);
            })?;
            next_frame += Duration::from_millis(50);
        }

        // Another host task gives ConTime exactly one processing opportunity.
        let progressed = contime.process()?;
        for rejected in contime.take_rejections() {
            eprintln!("Rejected: {rejected:?}");
        }
        // Other application tasks can run here on every loop turn.
        if !progressed {
            // Optional host idle policy; neither ConTime processing method waits.
            std::thread::yield_now();
        }
    }
    Ok(())
}
