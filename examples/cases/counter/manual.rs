//! Two counters driven by a host loop, without ConTime background threads.
//!
//! Included as a complete code sample in the Manual Rust documentation.
use contime::checkpoints::{CheckpointConfig, Timestamp};
use contime::manual::{ManualConTime, ManualConfig};
use contime::{AdvanceTime, ContimeEvent, ContimeModule, ContimeSnapshot};
use std::{
    cell::Cell,
    num::NonZeroUsize,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);
impl Timestamp for Time {
    fn minimum() -> Self { Self(0) }
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool { true }
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}
#[derive(ContimeEvent)]
#[contime_event(id = |event| event.id)]
struct Count {
    id: u128,
    time: Time,
    counters: Vec<u128>,
}
#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |id| *id, events = [Count => each(self.counters, |id| [*id])], apply = {
    self.value += batch.events.count() as u64;
})]
struct Counter {
    id: u128,
    time: Time,
    value: u64,
}
#[derive(ContimeModule)]
#[contime_module(time = Time, events = [Count], snapshots = [Counter])]
struct Counters;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut contime = ManualConTime::<Counters>::new(
        ManualConfig { queue_capacity: NonZeroUsize::new(128).unwrap(), checkpoints: CheckpointConfig { interval: 100 } },
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
            contime.apply([Count { id: frame, time, counters: if frame % 2 == 0 { vec![1, 2] } else { vec![1] } }.into()])?;
            contime.advance_to(time)?;
            let rendered = rendered.clone();
            contime.query_at(time, [Counter::snapshot_id(1), Counter::snapshot_id(2)], move |states| {
                let mut values = states.into_iter().map(|state| {
                    let CountersSnapshotLanes::Counter(state) = *state;
                    (state.id,state.value)
                }).collect::<Vec<_>>();
                values.sort_unstable_by_key(|(id,_)| *id);
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
