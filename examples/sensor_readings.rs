//! Request-scoped insertion completion and historical sensor readings.
//!
//! Run `cargo run -p contime --example sensor_readings` from the workspace.

use std::time::Duration;

use contime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime::{AdvanceTime, ConTime, ConTimeConfig, Input, Placement};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}

struct Reading {
    id: u128,
    sensor: u128,
    time: Time,
    temperature: i16,
}
impl Event for Reading {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl Input for Reading {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.sensor);
    }
}

#[derive(Clone, Default)]
struct Temperature {
    time: Time,
    value: Option<i16>,
}
impl Snapshot for Temperature {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl ApplyEvents<Reading> for Temperature {
    fn create(_: u128, _: &Reading) -> Self {
        Self::default()
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Reading>) {
        for reading in batch.events {
            self.value = Some(reading.temperature);
        }
    }
}

fn main() -> Result<(), String> {
    let history = ConTime::<Reading, Temperature, ()>::start(
        ConTimeConfig {
            router_count: 1,
            worker_count: 2,
            placement: Placement::default(),
            pruning_interval: Duration::from_millis(100),
            history_retention: Time(100),
            checkpoints: CheckpointConfig { interval: 2 },
        },
        (),
    )
    .map_err(|error| format!("{error:?}"))?;

    let (responses, inserted) = crossbeam_channel::unbounded();
    history
        .send(
            [
                Reading { id: 1, sensor: 7, time: Time(5), temperature: 17 },
                Reading { id: 2, sensor: 7, time: Time(10), temperature: 20 },
                Reading { id: 3, sensor: 7, time: Time(7), temperature: 18 },
            ],
            responses,
        )
        .map_err(|error| format!("{error:?}"))?;
    // Sender closure acknowledges insertion on every route, not live replay.
    let rejections = inserted.iter().collect::<Vec<_>>();
    assert!(rejections.is_empty(), "unexpected rejections: {rejections:?}");

    // Historical reads can reconstruct admitted future inputs before advancement.
    for (time, expected) in [(6, 17), (8, 18), (11, 20)] {
        let snapshots = history.query_at(Time(time), [7]).map_err(|error| format!("{error:?}"))?;
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].value, Some(expected));
        println!("Temperature at t={time}: {expected}");
    }
    let events = history.query_events_between(7, Time(5), Time(10)).map_err(|error| format!("{error:?}"))?;
    assert_eq!(events.iter().map(|event| event.event_id()).collect::<Vec<_>>(), [1, 3]);

    history.advance_to(Time(10)).map_err(|error| format!("{error:?}"))?;
    history.wait_until_idle(Duration::from_secs(2)).map_err(|error| format!("{error:?}"))?;
    assert!(history.errors().is_empty());
    history.shutdown();
    Ok(())
}
