# ConTime Simulator

## Generate an event stream

`EventStream` is a lazy iterator configured with an event count, a seed, and your
own settings. Supply two functions: one creates an event; the other mutates its
snapshot-identifying fields. Both receive the configuration and seeded randomness.
No batches or target objects are required.

```rust
use contime_simulator::{EventStream, StreamConfig};
use std::num::NonZeroU64;

struct Settings { snapshots: NonZeroU64 }
struct CounterChanged { counter_id: u64, delta: u64 }

let config = StreamConfig {
    event_count: 1_000,
    seed: 42,
    settings: Settings { snapshots: NonZeroU64::new(100).unwrap() },
};
let events = EventStream::new(
    config,
    |_, random| CounterChanged {
        counter_id: 0,
        delta: 1 + random.below(NonZeroU64::new(5).unwrap()),
    },
    |config, random, event| {
        event.counter_id = random.below(config.settings.snapshots);
    },
);

let mut counts = [0_u64; 100];
for event in events {
    // Submit the completed event to ConTime, or inspect it independently.
    counts[event.counter_id as usize] += event.delta;
}
assert!(counts.iter().sum::<u64>() >= 1_000);
```

The assignment function owns the ID range and distribution. Uniform random
selection does not guarantee coverage or equal counts. Your settings can contain
hotspot preferences, average/maximum snapshots per event, logical duration, or
application-specific identifying fields. Your callbacks interpret and validate
those settings, including any uniqueness requirements for multi-snapshot events.
The stream does not hash IDs or inspect lanes. Compute content-derived event IDs
after assignment, or derive them on demand from the completed event.

`Random` uses fixed SplitMix64 integer arithmetic, with unbiased `below` sampling.
It is suitable for repeatable simulation, not security. Creation and assignment
have separate random streams: changing payload generation does not shift snapshot
selection. Reproduction requires the same seed, settings, and deterministic
callbacks/captured state. New streams restart their sequences, so the application
owns event ID uniqueness across runs.

The iterator creates no threads, reads no clock, and does not sleep, pace, submit,
query, validate state, or allocate event buffers. Application callbacks may do
their own allocations. Event timestamps are callback-owned; a requested timeframe
in your settings does not itself limit wall-clock production. Callbacks may retain
sequence state. Zero events invokes neither callback. Fallible events can be
represented as `Result` items; use ordinary iterator handling to stop on errors.

Run the standalone hotspot example with:

```sh
cargo run -p contime-simulator --example event_stream
```

## Adaptive simulation API

The older `Simulator`, `Workload`, and `Maximum` interfaces below remain available
for existing consumers. They are separate from the independent event iterator.

Test a ConTime setup with generated events, independent query validation, and
adaptive load. The simulator uses a clock supplied by its caller and creates no
threads or timers. It works around Runtime, Manual, and WebAssembly hosts; the
host still owns execution, queries, retention, and shutdown.

`contime-monitor` records submitted and query-confirmed source events and target
applications. `contime-simulator` uses those observations to decide how much new
work to admit. Neither tool changes ConTime's ordering or replay rules.

## Maximum production

`Maximum` paces work with integer credits. The initial rate is independent of
query frequency. Its default burst holds one millisecond of work, or one whole
maximum-size event, whichever is larger. Its outstanding-work window covers the
expected query interval and response budget, with a hard safety ceiling.

A fresh query confirming progress can increase the rate by 10% if admission was
constrained. A slow query or work remaining from a previous sampled period
reduces it by 20%. Rate changes have a minimum step of one unit/second. Each query can trigger only one adjustment. An active query
exceeding the response budget pauses admission immediately; missing observations
also pause it. The default response budget is an absolute 50 ms, including host
scheduling, insertion gates, response polling and validation. Applications can
choose a different budget for their environment. Slow initial runs therefore
back off instead of repeatedly doubling a backlog.

Call `allowance(elapsed, &monitor)` before creating work, submit only a whole
event that fits, then debit its cost with `consume` and record its actual totals.
Keep calling it while queries are pending, even when no new work is generated.
Admission never sleeps. The host yields, processes its Manual instance, or runs
other tasks between calls. Set `minimum_work` to the largest event cost. Units
are target applications by default; `WorkUnit::Events` supports source-event
pacing while the monitor continues to record both totals.

These are workload-control settings, not ConTime execution settings. Query
confirmation can reconstruct a historical state without completing background
live replay. Do not label these observations as measured worker CPU throughput.

## Your events and snapshots

Implement `Workload` to supply your event generator, expected state and validator.
No snapshot counter, hash, ID format or timestamp type is required. `Simulator`
combines that model with Maximum and a monitor; it does not start ConTime.

```rust
use contime_simulator::{MaximumConfig, Simulator, Workload};
use std::{num::NonZeroU64, time::Duration};

#[derive(Default)]
struct Counter { expected: i64 }
impl Workload for Counter {
    type Event = i64;
    type Snapshot = i64;
    type Expectation = i64;
    type Error = &'static str;

    fn next_applications(&self) -> NonZeroU64 { NonZeroU64::new(1).unwrap() }
    fn generate(&mut self, _sequence: u64, _at: Duration) -> i64 {
        self.expected += 1;
        1
    }
    fn expectation(&self) -> i64 { self.expected }
    fn validate(&self, expected: &i64, actual: &[i64]) -> Result<(), Self::Error> {
        if actual == [*expected] { Ok(()) } else { Err("counter differs") }
    }
}

let mut simulation = Simulator::new(Counter::default(), MaximumConfig::default())?;
let mut state = 0;
// Replace this closure with submission to your own ConTime instance.
assert!(simulation.produce(Duration::ZERO, |increment| {
    state += increment;
    Ok::<_, ()>(())
}).unwrap());
let query = simulation.start_query(Duration::ZERO)?;
// Replace this slice with the complete result of the captured-prefix query.
simulation.complete_query(query, &[state], Duration::from_millis(1)).unwrap();
let metrics = simulation.sample(Duration::from_millis(1))?;
assert_eq!(metrics.confirmed.events, 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The host must query the exact submitted prefix represented by the captured
expectation and validate all relevant snapshots. Delayed or rejected admission
must be handled by the host before confirmation. A failed submission makes a
`Simulator` unusable because generation may have changed its expected model;
a failed validation confirms no work and should terminate the run.

Call `sample` periodically for bounded recent/average rates and outstanding-work
history. Sampling and rendering are separate from query frequency. For a
custom host loop, use `Monitor` and `Maximum` directly.
Numeric fixed-rate workloads can keep their own producer pacing and use only the
monitor. The counter's domain model and UI stay under `examples/`; shared policy
lives here.
