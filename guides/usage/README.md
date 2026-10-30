# Using ConTime

[Overview](../../README.md) · [How it works](../how-it-works/README.md) · [Workspace](../workspace/README.md)

This guide walks through the complete counter program, from defining its data to
running and querying it. Start with the short [API overview](../../README.md#using-contime)
if you just want to see the shape of the API.

A running application usually submits events as they arrive and queries state when it needs to display or use it. It does not wait for every submission to finish before doing anything else.

This walkthrough starts an in-memory ConTime instance with four workers. A producer submits counter changes at varying intervals, while the main loop queries and prints two counters **20 times per second**. Some events change both counters at once. The demo runs for five seconds, then stops its producer and shuts down ConTime.

The Rust blocks show the underlying trait contracts as one complete program. A version using the module derive with the same behavior is available as [`examples/cases/counter/readme.rs`](https://github.com/humacss/contime/blob/main/examples/cases/counter/readme.rs). Run it from a ConTime checkout with:

```bash
cargo run -p contime --example readme --features macros
```

## Dependencies and Features

To use ConTime 0.1.2 in your own Rust binary project, add:

```toml
[dependencies]
contime = "0.1.2"
fastrand = "2"
```

ConTime supports Rust 1.90 or newer. Runtime and built-in integer time support (`time`) are enabled by default, so ordinary
applications need no feature list. Enable `manual` for caller-driven processing,
`monitor` for monitoring, or `simulator` for traffic generation and feedback tools.
The [module API](../modules/README.md) starts managed processing with
`ConTime::<App>::start(config)`, or caller-driven processing with
`contime::manual::ManualConTime::<App>::new(config)`. The explicit-trait walkthrough below
uses `RawConTime` and `manual::RawManualConTime` instead. Call `process()` from
your own loop when using Manual.
Creating a Manual instance starts no threads. `process_with_clock` is an optional
helper for budgeted processing. The [manual crate](../../crates/manual/README.md)
describes its worker contracts and processing budget. Shared contracts and storage
adapters are available through `contime::core`. `fastrand` only varies the example
traffic and is not required by ConTime. For local development, you can instead use
`contime = { path = "/path/to/contime" }`.

The optional `monitor` feature automatically records worker-owned insertion and live replay
application totals. Without monitoring, this bookkeeping is compiled out. Totals and
observed rates are available through worker status; one source event targeting
three snapshots counts as three applications. Queries and retention reconstruction
are excluded. See the [monitor guide](../../crates/monitor/README.md) for counter and rate semantics.

The optional `monitor` feature exposes `contime::monitor`. With Runtime enabled,
`WorkerMonitor::attach(&contime)` collects read-only worker reports without
reconstructing snapshots. Collection and rate calculation are separate from
presentation; the crate has no UI dependencies. Reports are observations of
individual workers, not a synchronized global snapshot or completion fence.

The independent optional `simulator` feature exposes `contime::simulator`. Its seeded
`EventStream` iterator creates events and assigns their snapshot-identifying
fields through two caller-provided functions. The crate also provides
caller-driven adaptive admission and query validation tools when `monitor` is
also enabled. Event generation alone does not enable worker bookkeeping. It starts no threads and changes no execution or replay rules.
See the [simulator guide](../../crates/simulator/README.md) for the APIs and examples.

## Declarative application modules

Enable `macros` to assemble trait-based events, snapshots and nested modules
into flat lanes with `ContimeModule`. Start with the [module walkthrough](../modules/README.md)
and `cargo run --example modules --features macros`. The explicit trait walkthrough
below remains useful when adapting existing application types.

## Define Time

First, import ConTime and the traits used by the example:

```rust
use std::thread;
use std::time::{Duration, Instant};

use contime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime::{AdvanceTime, RawConTime, ConTimeConfig, Input, Placement};
```

ConTime uses your application's time type. Here, `Time` counts logical milliseconds. The application will calculate it from elapsed time; ConTime does not own a clock. The type provides ordering, an explicit minimum, a checked predecessor, and retention subtraction. `Default`
is an ordinary construction value, not the history boundary. Built-in integer
types already implement these contracts when the default `time` feature is enabled;
signed integers support negative timestamps down to their minimum.

```rust
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct Time(u64);

impl Timestamp for Time {
    fn minimum() -> Self { Self(u64::MIN) }

    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool { true }

    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}
```

## Define an Event and Its History Keys

`CounterChanged` adds `delta` to each counter listed in `counter_ids` at the supplied time. `Event` exposes its timestamp; `Input` supplies its event ID and the snapshot histories it affects.

```rust
struct CounterChanged {
    event_id: u128,
    counter_ids: Vec<u128>,
    time: Time,
    delta: i64,
}

impl Event for CounterChanged {
    type Time = Time;

    fn time(&self) -> Time {
        self.time
    }
}

impl Input for CounterChanged {
    fn event_id(&self) -> u128 {
        self.event_id
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for &counter_id in &self.counter_ids {
            emit(counter_id);
        }
    }
}
```

You choose how to generate IDs. Event IDs are usually derived from the event content so retries reproduce the same ID. This example assigns increasing IDs instead. Snapshot IDs identify histories and stay the same as their state changes; they can be derived from identifying fields or assigned manually. Here, histories 1 and 2 represent the two counters.

An event targeting `[1, 2]` contributes to both histories. Each history is updated independently; routing one event to several counters does not make their updates a cross-worker transaction.

## Define Snapshot State and Application

`Counter` stores its history ID, timestamp, and current value. The ID lets the display identify returned snapshots without relying on their order. `create` starts each counter at zero, and `apply_events` adds each event's delta. ConTime updates the snapshot timestamp through `set_time`.

```rust
#[derive(Clone, Default)]
struct Counter {
    counter_id: u128,
    time: Time,
    value: i64,
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

impl ApplyEvents<CounterChanged> for Counter {
    fn create(snapshot_id: u128, _first_event: &CounterChanged) -> Self {
        Self { counter_id: snapshot_id, ..Self::default() }
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, CounterChanged>) {
        for event in batch.events {
            self.value += event.delta;
        }
    }
}
```

## Start ConTime

Start the in-memory ConTime instance inside `main`:

```rust
fn main() {
    let config = ConTimeConfig {
        router_count: 1,
        worker_count: 4,
        placement: Placement::default(),
        pruning_interval: Duration::from_millis(100),
        history_retention: Time(1_000),
        checkpoints: CheckpointConfig { interval: 32 },
    };
    let history = RawConTime::<CounterChanged, Counter, ()>::start(config, ())
        .expect("start ConTime");
```

`worker_count` selects the number of history-owning worker threads. Independent histories can be processed across those workers; a particular counter's history stays on one worker. This configuration also starts one router thread and an admission coordinator thread.

`history_retention` requests 1,000 logical milliseconds of history behind the processing target. `pruning_interval` controls wall-clock spacing between coordination rounds. `checkpoints` controls checkpoint spacing, and the default `placement` maps snapshot IDs to workers.

The type parameters select the event, snapshot, and application wrapper. The final `()` uses ordinary snapshot application without extra application hooks.

## Submit Changes Without Waiting

Continue inside `main`. The producer and display share the same start time. A scoped thread submits events every 40–180 milliseconds, with random positive increments. Every third event targets both counters; the others target one counter at a time.

```rust
    let start = Instant::now();
    let duration = Duration::from_secs(5);
    thread::scope(|scope| {
        scope.spawn(|| {
            let mut event_id = 0;
            loop {
                thread::sleep(Duration::from_millis(fastrand::u64(40..=180)));
                if start.elapsed() >= duration {
                    break;
                }
                event_id += 1;
                let counter_ids = match event_id % 3 {
                    0 => vec![1, 2],
                    1 => vec![1],
                    _ => vec![2],
                };
                let event = CounterChanged {
                    event_id,
                    counter_ids,
                    time: Time(start.elapsed().as_millis() as u64),
                    delta: fastrand::i64(1..=5),
                };
                history.apply([event]).expect("enqueue counter change");
            }
        });
```

`apply` queues work and returns without waiting for insertion or replay. Rejected inputs are reported separately through `errors()`, which the display loop drains below. There is no insertion response wait between producing events and rendering frames.

## Query and Display State at 20 Hz

The main thread advances the processing target and queries the current application time every 50 milliseconds. It prints the available state, then waits until the next frame deadline. Counters without a returned snapshot are displayed as zero until their first event becomes visible.

```rust
        let frame_interval = Duration::from_millis(50);
        let mut next_frame = start;
        while start.elapsed() < duration {
            let time = Time(start.elapsed().as_millis() as u64);
            history.advance_to(time).expect("advance processing target");
            let snapshots = history.query_at(time, [1, 2]).expect("query current counters");
            let values = [1, 2].map(|id| {
                snapshots.iter().find(|snapshot| snapshot.counter_id == id)
                    .map_or(0, |snapshot| snapshot.value)
            });
            println!("t={}ms counter 1={} counter 2={}", time.0, values[0], values[1]);
            for rejection in history.errors().try_iter() {
                eprintln!("rejected input: {rejection:?}");
            }
            next_frame += frame_interval;
            thread::sleep(next_frame.saturating_duration_since(Instant::now()));
        }
    });
```

Each query waits for its own results, not for the producer or all background processing to finish. It reconstructs admitted history through the requested time, including events exactly at that time. A recently submitted event may become visible on a later frame. The 50-millisecond deadline sets the target display cadence; actual timing also depends on query cost and operating-system scheduling.

The counters advance at varying intervals, and their values differ between runs. In a graphical application, the rendering step would use these snapshots to draw the frame rather than print a line.

## Shut Down

After five seconds, the display loop ends and the scoped producer exits. Once the producer is joined, close admission and join ConTime's owned threads:

```rust
    let report = history.shutdown();
    assert!(report.routers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
    assert!(report.workers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
}
```

The example uses `expect` and assertions to expose failures. Applications can handle submission errors, rejections, and shutdown outcomes according to their own policy. Idle waits are helpers for controlled tests and benchmarks; this running example never waits for global idle.

For a practical example with delayed client inputs and corrected historical paths, see [client movement](../../examples/cases/client_movement/README.md).

## Submissions and Completion

Submit one event or an iterator of events with `apply`; use `send` when you need a request-specific rejection stream and insertion response. ConTime manages internal transport batching independently: it splits large submissions and combines adjacent submissions already waiting in the queue. The isolated `contime-batcher` crate manages this packetization after admission and before routing. It never waits to fill a batch. Each request keeps its own rejection destination and completion ownership; internal packet boundaries do not change event timestamps or canonical replay order. Gathering is bounded so queries and coordination can make progress, but gathering and dispatch still consume CPU time.

## Run ConTime Manually

If your application already owns the loop, create a manual instance and decide
when it processes work. No ConTime background threads are started. Use the same
`Input`, `Snapshot`, and `ApplyEvents` contracts described below; you do not need
to define worker messages or storage adapters.

```toml
contime = { version = "0.1.2", features = ["manual"] }
```

The self-contained [Manual code sample](../../examples/cases/counter/manual.rs),
also included in the Rust documentation, drives two counters from a host loop
at 20 frames per second. To run the live counter with Manual processing:

```sh
cargo run --all-features --example counter -- --execution manual
```

For a constrained build that omits the managed runtime, monitoring, counters, and
simulator, explicitly select only Manual:

```toml
contime = { version = "0.1.2", default-features = false, features = ["manual", "time"] }
```

Keep `time` for built-in integer timestamps. With a custom time type implementing
the time traits, it can also be omitted.

Likewise, `default-features = false, features = ["runtime", "time"]` selects a lean
Runtime without worker counters or the monitoring and simulation tools. Feature
selection controls available code; enabling a tool never starts collection,
event generation, or background threads by itself.

The self-contained sample schedules submissions and display requests at 20 Hz, while giving
ConTime one processing opportunity on every loop turn. The key operations are:

```rust,ignore
contime.apply([event])?;
contime.advance_to(time)?;
contime.query_at(time, [1, 2], |snapshots| {
    // Update the displayed state here, when processing reaches this query.
})?;
contime.process()?;
```

`process()` returns `true` when it makes progress and `false` when nothing is
currently runnable. Each call handles one message, complete replay/pruning call,
or pruning-barrier transition; a step may apply several events. Other application
tasks run between calls. Query callbacks run on the calling thread during
processing, including for empty results, and may capture thread-local state.

`process_with_clock(budget, clock)` is an optional helper that repeats the same
step up to a maximum time/work allowance. It returns early when no work is ready
and never sleeps or waits to fill the allowance. The limit is soft: a complete
operation can overrun it because time is checked between operations.

Drain `take_rejections()` regularly; queue acceptance does not guarantee
insertion or completed replay. Retention is explicit through `prune_before`, and
its horizon must not exceed the queued processing target. The convenience API
currently provides snapshot/event queries without subscriptions.

The [WebAssembly example](../../examples/environments/wasi/README.md) runs outside a browser through WASI:
Manual on standard WASI, or Manual and Runtime on threaded WASI with a compatible host. The separate [browser example](../../examples/environments/browser/README.md)
uses Manual with simulator/monitor feedback for counters and a configurable dashboard; browser presentation stays under `examples/support/composition/browser/ui/`.
It drives processing through `requestAnimationFrame` and `performance.now()`.

## Explore Live Workloads

The [example guide](../../examples/README.md) lists runnable walkthroughs and configurable
live dashboards. Each scenario has its own folder and reader guide. Start with
its application model and ConTime loop; UI code lives separately under `ui/`.

```sh
cargo run --release --all-features --example counter
cargo run --release --all-features --example client_movement
cargo run --release --all-features --example counter -- --execution manual
```

Start the central example launcher with `cargo run --release --example launcher`
to select a terminal, standalone WebAssembly, or browser host, execution model,
application definition, and settings. See the [launcher guide](../../examples/support/launcher/README.md).

Live application definitions are shared across hosts: each example's `model.rs`
and `setup.rs` contain its event and snapshot contracts, while `examples/support`
contains load generation, execution and UI. Pass `--execution manual` to a native
live example to use caller-driven processing. All Cargo example targets compile
both backends and the monitoring/simulation tools; ordinary library feature
selection remains independent.
All live cases use the simulator and worker monitor
with cumulative per-worker throughput averages. WASI workload processing and full
validation stay inside WebAssembly; its UI runs in the launcher. Browser hosts
use Manual through a frame-budgeted adapter.

The counter and chain-hash dashboards offer startup settings for snapshot count, worker count (Runtime),
event rate, average/maximum snapshots per event, query frequency, retention and checkpoint spacing.
Numeric event rates also offer an outstanding-work allowance. Live demos default to one worker per available logical CPU, 1,000 snapshots
per worker, Maximum event production, and 128-event checkpoint spacing. Manual
uses one worker. Choose a numeric event-rate cap for a controlled load. The
production window bounds the example's own backlog and reference data. Maximum
adjusts offered load and manages that window from worker status; a numeric rate
retains a configurable source-event allowance. Worker readings confirm bounded
production windows independently of state queries. Dashboards show target and
cumulative completed source-event rates, unconfirmed events, and per-worker
application totals and cumulative averages. Hashes validate state at completed
windows; counters validate at shutdown.

Client movement is a real-time case with its own client count, input
frequency, latency/jitter and fading settings. It queries current and historical
character positions to show corrected paths. The launcher uses the same Bevy
presentation on desktop and in the browser; direct native runs retain a terminal map. It uses a fixed
input cadence rather than Maximum production; keyboard inputs follow the same
delayed delivery path as simulated clients. See the
[movement guide](../../examples/cases/client_movement/README.md).
Use `--headless --duration 5` for finite comparisons without terminal rendering.
The browser demo exposes one Manual worker and a configurable frame allowance.
Measurements include example overhead; these are live diagnostics rather than
isolated ConTime benchmarks.

## Verify Order-Sensitive State

The separate [`chain_hash` example](https://github.com/humacss/contime/blob/main/examples/support/entrypoints/chain_hash.rs) runs an interactive dashboard that checks a hash chain for each history. Every new digest depends on the previous digest and the event, so it carries the contribution of the entire accepted sequence. Checkpoints preserve the digest when older events are pruned. Unlike addition, the result depends on event order.

```bash
cargo run -p contime --all-features --example chain_hash -- --snapshots 6 --workers 4 --events-per-second 100
```

The example submits event groups in reverse order and queries fully admitted timestamp ranges. Its dashboard shows the target and average completed source-event rates, monitor-unconfirmed work, worker activity, and last verified and latest produced hashes. Each queried digest must match its captured canonical reference state; an invalid result stops the example with an error. It runs until `q`, Escape, or Ctrl-C. See the [example guide](https://github.com/humacss/contime/blob/main/examples/cases/chain_hash/README.md) for configuration and verification details. The counter walkthrough above needs no hashing or terminal UI dependencies.

## Reproduce the reconciliation recording

The README recording uses an opt-in, repeatable choreography rather than random
traffic. Five clients follow compact paths, with bots in four separate quadrants and the
player near the center, with one action per second and
staggered sends. Explicit delays from 150 to 1,700 ms regularly deliver newer
turns before older ones. Bots keep moving; the player's square includes a rest
to demonstrate Stop. All events still go through the same simulated delivery,
ConTime processing, validated queries and renderer as the interactive example.
No queried state or corrections are fabricated.

Each client has a consistent color across the timeline, inputs, delivery animations
and queried paths. Gray paths show earlier rendering states that now disagree;
double delivery ticks mark out-of-order arrivals. The timeline spans six seconds,
with unlabeled one-second divisions. The browser displays only the shared graphical
view. The canvas is recorded directly at 60 fps, then a ten-second excerpt is
converted to a 50 fps GIF for the README, avoiding repeated screenshots during
simulation.

To reproduce the choreography, first launch the browser host below, then add
`showcase=1&client_spacing=18&movement_speed=8` to its URL, replacing any existing
spacing and speed values. Five clients are required. The scripted inputs ignore
keyboard movement, and after the initial two-second positioning, the complete sequence repeats every 24 seconds. Omit
`showcase=1` to return to the normal interactive generator.

With the [browser prerequisites](../../examples/environments/browser/README.md#build-directly)
installed, run from the repository root:

```sh
cargo run --release --example launcher -- \
  --environment browser --execution manual --example client-movement --budget-ms 1 --no-setup -- \
  --clients 5 --client-spacing 16 --movement-speed 12 --input-hz 1 --query-hz 20 \
  --average-latency-ms 1000 --jitter-ms 1000 --trail-ms 5000 --fade-ms 5000
```

Click the map, then use WASD or the arrow keys to choose a direction. Space stops
your character. Keyboard inputs travel through the same simulated delivery delay
as bot inputs. Omit a duration to run until you close the page or stop the launcher.
