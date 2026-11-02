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

#[derive(ContimeEvent)]
struct Count {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: Time,
    counters: Vec<u128>,
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |counter| counter.id, map = counter_key)]
#[contime(event = Count, id = counter_event_ids)]
struct Counter {
    id: u128,
    #[contime(time)]
    time: Time,
    value: u64,
}

fn counter_key(id: u128) -> u128 {
    contentid::typed_content_id_u128(std::any::type_name::<Counter>(), &id)
}

fn counter_event_ids(event: &Count, emit: &mut dyn FnMut(u128)) {
    for id in &event.counters {
        emit(counter_key(*id));
    }
}

impl contime::SnapshotCreate for Counter {
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        let Some(event) = event.get::<Count>() else { unreachable!("the builder registers Count") };
        let Some(id) = event.counters.iter().copied().find(|id| counter_key(*id) == snapshot_id) else {
            unreachable!("routing selected one of these counters")
        };
        Self { id, ..Self::default() }
    }
}
impl SnapshotApply for Counter {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        self.value += batch.events::<Count>().count() as u64;
    }
}

#[derive(ContimeModule)]
#[contime(snapshots = [Counter])]
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
            contime.query_at(time, [counter_key(1), counter_key(2)], move |states| {
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
