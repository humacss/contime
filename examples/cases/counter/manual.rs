//! Two counters driven by a host loop, without ConTime background threads.
//!
//! Run `cargo run --no-default-features --features manual --example manual_counter`.
use contime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime::manual::{ManualConTime, ManualConfig};
use contime::{AdvanceTime, Input};
use std::{
    cell::Cell,
    num::NonZeroUsize,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl AdvanceTime for Time {
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}
struct Count {
    id: u128,
    time: Time,
    counters: Vec<u128>,
}
impl Event for Count {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl Input for Count {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for id in &self.counters {
            emit(*id)
        }
    }
}
#[derive(Clone, Default)]
struct Counter {
    id: u128,
    time: Time,
    value: u64,
}
impl Snapshot for Counter {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl ApplyEvents<Count> for Counter {
    fn create(id: u128, _: &Count) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Count>) {
        self.value += batch.events.count() as u64;
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut contime = ManualConTime::<Count, Counter>::new(
        ManualConfig { queue_capacity: NonZeroUsize::new(128).unwrap(), checkpoints: CheckpointConfig { interval: 100 } },
        (),
    );
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
            contime.apply([Count { id: frame, time, counters: if frame % 2 == 0 { vec![1, 2] } else { vec![1] } }])?;
            contime.advance_to(time)?;
            let rendered = rendered.clone();
            contime.query_at(time, [1, 2], move |mut states| {
                states.sort_by_key(|state| state.id);
                let values = states.iter().map(|state| (state.id, state.value)).collect::<Vec<_>>();
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
