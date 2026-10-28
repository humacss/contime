[![Crates.io](https://img.shields.io/crates/v/contime.svg)](https://crates.io/crates/contime)
[![Docs](https://docs.rs/contime/badge.svg)](https://docs.rs/contime)
[![CI](https://github.com/humacss/contime/actions/workflows/ci.yml/badge.svg)](https://github.com/humacss/contime/actions/workflows/ci.yml)
[![Qlty | Maintainability](https://qlty.sh/gh/humacss/projects/contime/maintainability.svg)](https://qlty.sh/gh/humacss/projects/contime)
[![Qlty | Code Coverage](https://qlty.sh/gh/humacss/projects/contime/coverage.svg)](https://qlty.sh/gh/humacss/projects/contime)

# contime

> **Work in progress:** ConTime is still being developed for my own use. There is more cleanup to do, and the API needs further refinement before it is ready for general adoption. You are welcome to try it, but expect changes and rough edges.

`contime` is an in-memory Rust workspace that builds queryable continuous-time state from unreliable event streams.

It handles delayed, duplicated, and out-of-order events, and distributes independent snapshot histories across workers. The root `contime` crate re-exports managed or caller-driven processing. [`contime-core`](https://github.com/humacss/contime/blob/main/crates/core/README.md) provides shared application contracts and synchronous storage; the other crates provide the individual parts.

## Continuous Time?

Most systems today work with **discrete state**: "**right now** the state is X".

`contime` takes a different approach: it builds and maintains **continuous-time state**.

This means that state is defined **at every point in time**, not only at the moments when events arrive, but also in between those events, before those events, and after those events, within the retained history.

Now you can ask the question, what was the state at time `T`?

This also helps handle out-of-order events cleanly. What if an event comes in late?

Many systems will struggle to handle this because all you have is the current state. You don't know what the state looked like at the time the event was supposed to arrive.

With continuous-time state, applying an event from the past is as natural as applying one in the present. Histories order retained events by time and event ID, so the result does not depend on their arrival order. Your event application logic supplies the deterministic state changes.

## Target Use Cases

`contime` was originally built for the needs of a high-performance authoritative game server, but it is generally useful for systems that require historical state and out-of-order event processing.

Typical good fits include:

**Multiplayer game servers and clients**  
Maintain deterministic historical state for delayed, duplicated, or out-of-order player inputs.

**IoT & sensor network data**  
Apply readings from unreliable devices and networks into a consistent, queryable timeline.

**Audit trails & lightweight event-sourcing**  
Provide time-travel queries without needing a full persistent event store.

## Using `contime`

A running application usually submits events as they arrive and queries state when it needs to display or use it. It does not wait for every submission to finish before doing anything else.

Submit one event or an iterator of events with `apply`; use `send` when you need a request-specific rejection stream and insertion response. ConTime manages internal transport batching independently: it splits large submissions and combines adjacent submissions already waiting in the queue. The isolated `contime-batcher` crate manages this packetization after admission and before routing. It never waits to fill a batch. Each request keeps its own rejection destination and completion ownership; internal packet boundaries do not change event timestamps or canonical replay order. Gathering is bounded so queries and coordination can make progress, but gathering and dispatch still consume CPU time.

This walkthrough starts an in-memory ConTime engine with four workers. A producer submits counter changes at varying intervals, while the main loop queries and prints two counters **20 times per second**. Some events change both counters at once. The demo runs for five seconds, then stops its producer and shuts down ConTime.

The Rust blocks form one complete program, in order. The same program is available as [`examples/cases/counter/main.rs`](https://github.com/humacss/contime/blob/main/examples/cases/counter/main.rs). Run it from a ConTime checkout with:

```bash
cargo run -p contime --example counter
```

### Dependencies and Features

To use ConTime 0.1 in your own Rust binary project, add:

```toml
[dependencies]
contime = "0.1.0"
fastrand = "2"
```

ConTime supports Rust 1.90 or newer. The default `runtime` feature starts managed background threads. For caller-driven processing, use `contime = { version = "0.1", default-features = false, features = ["manual"] }` and create a `contime::manual::ManualConTime` instance. Creating that instance starts no threads; your application calls `process()` between its other work, with an optional `process_with_clock` helper for budgeted processing. The [manual crate](crates/manual/README.md) describes its worker contracts and processing budget. Both features may be enabled together. Shared application and storage adapters are available through `contime::core`. `fastrand` only varies the example's event intervals and increments; it is not required by ConTime. For local development, you can instead use `contime = { path = "/path/to/contime" }`.

The optional `counters` feature records worker-owned insertion and live replay
application totals. It is disabled by default. With `monitor` enabled, totals and
observed rates are available through worker status; one source event targeting
three snapshots counts as three applications. Queries and retention reconstruction
are excluded. See the [monitor guide](crates/monitor/README.md) for counter and rate semantics.

The optional `monitor` feature exposes `contime::monitor`. With Runtime enabled,
`WorkerMonitor::attach(&contime)` collects read-only worker reports without
reconstructing snapshots. Collection and rate calculation are separate from
presentation; the crate has no UI dependencies. Reports are observations of
individual workers, not a synchronized global snapshot or completion fence.

The optional `simulator` feature exposes `contime::simulator`. Its seeded
`EventStream` iterator creates events and assigns their snapshot-identifying
fields through two caller-provided functions. The crate also provides
caller-driven adaptive admission and query validation tools using monitor
observations. It starts no threads and changes no execution or replay rules.
See the [simulator guide](crates/simulator/README.md) for the APIs and examples.

### Run ConTime Manually

If your application already owns the loop, create a manual instance and decide
when it processes work. No ConTime background threads are started. Use the same
`Input`, `Snapshot`, and `ApplyEvents` contracts described below; you do not need
to define worker messages or storage adapters.

```toml
contime = { version = "0.1", default-features = false, features = ["manual"] }
```

The complete [manual counter example](examples/cases/counter/manual.rs) drives two
counters from a host loop at 20 frames per second:

```sh
cargo run --no-default-features --features manual --example manual_counter
```

Its loop schedules submissions and display requests at 20 Hz, while giving
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

The [WebAssembly example](examples/environments/wasi/README.md) runs outside a browser through WASI:
Manual on standard WASI, or Manual and Runtime on threaded WASI with a compatible host. The separate [browser example](examples/environments/browser/README.md)
uses Manual with simulator/monitor feedback for counters and a configurable dashboard; browser presentation stays under `examples/support/composition/browser/ui/`.
It drives processing through `requestAnimationFrame` and `performance.now()`.

### Define Time

First, import ConTime and the traits used by the example:

```rust
use std::thread;
use std::time::{Duration, Instant};

use contime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime::{AdvanceTime, ConTime, ConTimeConfig, Input, Placement};
```

ConTime uses your application's time type. Here, `Time` counts logical milliseconds. The application will calculate it from elapsed time; ConTime does not own a clock. The type provides ordering, a default starting time, a checked predecessor, and retention subtraction.

```rust
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
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
```

### Define an Event and Its History Keys

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

### Define Snapshot State and Application

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

### Start ConTime

Start the in-memory engine inside `main`:

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
    let history = ConTime::<CounterChanged, Counter, ()>::start(config, ())
        .expect("start ConTime");
```

`worker_count` selects the number of history-owning worker threads. Independent histories can be processed across those workers; a particular counter's history stays on one worker. This configuration also starts one router thread and an admission coordinator thread.

`history_retention` requests 1,000 logical milliseconds of history behind the processing target. `pruning_interval` controls wall-clock spacing between coordination rounds. `checkpoints` controls checkpoint spacing, and the default `placement` maps snapshot IDs to workers.

The type parameters select the event, snapshot, and application wrapper. The final `()` uses ordinary snapshot application without extra application hooks.

### Submit Changes Without Waiting

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

### Query and Display State at 20 Hz

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

### Shut Down

After five seconds, the display loop ends and the scoped producer exits. Once the producer is joined, close admission and join ConTime's owned threads:

```rust
    let report = history.shutdown();
    assert!(report.routers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
    assert!(report.workers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
}
```

The example uses `expect` and assertions to expose failures. Applications can handle submission errors, rejections, and shutdown outcomes according to their own policy. Idle waits are helpers for controlled tests and benchmarks; this running example never waits for global idle.

For a practical example with delayed client inputs and corrected historical paths, see [client movement](examples/cases/client_movement/README.md).

### Explore Live Workloads

The [example guide](examples/README.md) lists runnable walkthroughs and configurable
live dashboards. Each scenario has its own folder and reader guide. Start with
its application model and ConTime loop; UI code lives separately under `ui/`.

```sh
cargo run --release --features monitor,counters --example counter_live
cargo run --release --features monitor,counters --example client_movement
cargo run --release --no-default-features --features manual,monitor,counters --example manual_counter_live
```

Start the central example launcher with `cargo run --release --example launcher`
to select a terminal, standalone WebAssembly, or browser host, execution model,
application definition, and settings. See the [launcher guide](examples/support/launcher/README.md).

Live application definitions are shared across hosts: each example's `model.rs`
and `setup.rs` contain its event and snapshot contracts, while `examples/support`
contains load generation, execution and UI. Add `--features manual` and pass
`--execution manual` to a native live example to use caller-driven processing.
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

Client movement is a native real-time case with its own client count, input
frequency, latency/jitter and fading settings. It queries current and historical
character positions to show corrected paths on a terminal map. It uses a fixed
input cadence rather than Maximum production; keyboard inputs follow the same
delayed delivery path as simulated clients. See the
[movement guide](examples/cases/client_movement/README.md).
Use `--headless --duration 5` for finite comparisons without terminal rendering.
The browser demo exposes one Manual worker and a configurable frame allowance.
Measurements include example overhead; these are live diagnostics rather than
isolated engine benchmarks.

### Verify Order-Sensitive State

The separate [`chain_hash` example](https://github.com/humacss/contime/blob/main/examples/support/entrypoints/chain_hash.rs) runs an interactive dashboard that checks a hash chain for each history. Every new digest depends on the previous digest and the event, so it carries the contribution of the entire accepted sequence. Checkpoints preserve the digest when older events are pruned. Unlike addition, the result depends on event order.

```bash
cargo run -p contime --features monitor,counters --example chain_hash -- --snapshots 6 --workers 4 --events-per-second 100
```

The example submits event groups in reverse order and queries fully admitted timestamp ranges. Its dashboard shows the target and average completed source-event rates, monitor-unconfirmed work, worker activity, and last verified and latest produced hashes. Each queried digest must match its captured canonical reference state; an invalid result stops the example with an error. It runs until `q`, Escape, or Ctrl-C. See the [example guide](https://github.com/humacss/contime/blob/main/examples/cases/chain_hash/README.md) for configuration and verification details. The counter walkthrough above needs no hashing or terminal UI dependencies.

## How it Works

ConTime builds continuous-time state from two concepts you define: **snapshots**, which represent state, and **events**, which describe changes to that state.

You define the data and how events change it. ConTime maintains the history needed to reconstruct that state at a requested time.

### Snapshots

A snapshot represents state at a particular point in time.

For example, a snapshot could describe a sensor's latest reading. The snapshot type defines what data is stored, while a snapshot ID identifies which sensor the data belongs to.

ConTime maintains an independent history for each snapshot ID. The same sensor can therefore have different snapshot state at different times, and you can ask for its state at time `T`.

### Events

An event contains timestamped data that can change one or more snapshots. For example, a new sensor reading updates the state belonging to that sensor.

Your input implementation gives the event an ID, a timestamp, and the snapshot IDs it should affect. Your application logic defines how the event changes those snapshots.

An event can affect several histories. A sensor reading might update both the individual sensor's state and a shared summary of readings from multiple sensors.

### Applying Events

Applying events means taking snapshot state and updating it using the events that belong at that time.

Suppose a sensor's state contains its latest reading and the number of readings received. Your application logic might replace the stored reading and increment the count whenever a new reading arrives.

ConTime supplies events in chronological order, grouped into complete timestamp batches. Your application logic processes each batch to produce the next snapshot state. Events at the same timestamp are ordered by event ID.

Events do not need to arrive in that order. If a reading arrives late, ConTime reconstructs the affected history with that reading in its proper place, then reapplies the later events. Previously calculated state may change as a result.

Resubmitting an identical event with the same retained event ID does not apply it twice. Given the same retained events, stable IDs, and deterministic application logic, the resulting state is independent of arrival order.

### Queries

You generally use ConTime by submitting events and querying the state they describe.

A query asks for one or more snapshots at a particular time. For example: what was sensor 42's state at time `T`?

ConTime reconstructs the requested state by applying the relevant events through `T`, including events exactly at that timestamp. Events after `T` do not contribute to the result.

The result describes state at the requested time. Querying an earlier time does not rewind the running engine or replace its current state.

Queries use admitted, retained history. Submitting an event is asynchronous, so a query does not guarantee that earlier submissions have finished processing. Use the completion response or progress notification corresponding to the operation you need to observe. Insertion acknowledgements establish insertion, not completed live replay.

### Checkpoints

Replaying an entire history for every query would become increasingly expensive. ConTime keeps **checkpoints** to avoid that repeated work.

A checkpoint is saved snapshot state at a particular time. It provides a starting point for reconstructing later state.

To answer a query at time `T`, ConTime starts from an appropriate checkpoint and applies the remaining events through `T`. It only needs to replay the part of the history after that starting point.

If a late event changes history, checkpoints affected by that event must be recalculated. Earlier valid checkpoints can still be used.

Checkpoint spacing controls the tradeoff between storing more saved states and replaying more events. Checkpoints are an optimization; your event application logic still determines the state.

### Routers and Workers

ConTime distributes independent snapshot histories across workers so that they can be processed concurrently.

A **router** uses the destination snapshot IDs supplied by your input implementation to send work to the appropriate workers. All work belonging to a particular snapshot history goes to the same worker. An event affecting several histories can be routed to several workers.

A **worker** owns its assigned histories, applies their events, maintains their checkpoints, and serves queries for their state. Each worker runs on its own thread.

Workers process replay in bounded steps and service incoming messages between those steps. This lets a busy history make progress while other submissions and queries are also handled.

The overall flow is:

```text
submit events
    -> group events by destination snapshot
    -> admit and route the work
    -> workers update their histories

query snapshots at time T
    -> route each request to the owning worker
    -> reconstruct the requested state
    -> return the results
```

### Advancement and Retention

ConTime keeps a processing target that you move forward with `advance_to`.

That target controls how far live event application should progress. Future events can already be stored and queried before live processing reaches them. A query reconstructs state independently; it does not advance the processing target.

ConTime also retains a configurable amount of history. As time advances, it can discard older events and checkpoints once coordinated progress establishes that they are safe to prune.

Pruning preserves the snapshot state needed as a starting point for the remaining history. Queries cannot reconstruct history that has already been discarded.

Advancement requests are asynchronous. Processing observations and pruning notifications expose their respective progress boundaries. Idle waiting is a helper for tests and benchmarks, where input can be controlled; running applications should not rely on the system becoming idle.

## Workspace

| Crate | Responsibility |
| --- | --- |
| [`contime-monitor`](crates/monitor/README.md) | Read-only worker status, workload accounting, and bounded observation history |
| [`contime-simulator`](crates/simulator/README.md) | Caller-driven generation, independent validation, and paced Maximum admission |
| [`contime-batcher`](crates/batcher/README.md) | Bounded packetization and ready-queue draining for the native pipeline |
| [`contime-manual`](crates/manual/README.md) | Caller-driven processing of one worker between complete operations |
| `contime` (root) | Public facade and selection of manual or managed runtime processing |
| [`contime-core`](https://github.com/humacss/contime/blob/main/crates/core/README.md) | Shared application contracts and synchronous history/storage adapters |
| [`contime-api`](https://github.com/humacss/contime/blob/main/crates/api/README.md) | Input requests, responses, and optional local callback adapters |
| [`contime-router`](https://github.com/humacss/contime/blob/main/crates/router/README.md) | Deterministic routing to worker-owned histories |
| [`contime-runtime`](https://github.com/humacss/contime/blob/main/crates/runtime/README.md) | Managed instance, admission, batching integration, router/worker threads, and shutdown |
| [`contime-worker`](https://github.com/humacss/contime/blob/main/crates/worker/README.md) | History ownership and incremental replay scheduling |
| [`contime-snapshots`](https://github.com/humacss/contime/blob/main/crates/snapshots/README.md) | Checkpoint storage and historical reconstruction |
| [`contime-events`](https://github.com/humacss/contime/blob/main/crates/events/README.md) | Ordered retained event history |
| [`contime-lanes`](https://github.com/humacss/contime/blob/main/crates/lanes/README.md) | Consumer application and hook contracts |
| [`contime-progress`](https://github.com/humacss/contime/blob/main/crates/progress/README.md) | Participant progress measurement |

`contime-manual` owns caller-driven processing; `contime-batcher` owns transport batching for the native pipeline. Neither depends on the other.

Verify each concern independently:

```bash
cargo +1.90.0 test -p contime-manual
cargo +1.90.0 test -p contime-batcher
cargo +1.90.0 bench -p contime-batcher --bench batcher
cargo +1.90.0 test --manifest-path examples/environments/wasi/Cargo.toml
```

The [WebAssembly example](examples/environments/wasi/README.md) verifies Manual and threaded Runtime
execution through their respective WASI targets; the [browser example](examples/environments/browser/README.md) adds a
configurable browser host.

## Current Constraints

- ConTime is in-memory only. Persistence, replay from disk, and recovery belong in surrounding systems.
- Retention prunes old events and checkpoints by time. There is no built-in memory cap.
- Application logic must be deterministic. Stable event IDs and snapshot routing belong to the consumer.
- Queries earlier than retained history cannot reconstruct discarded events.
- Checkpoints clone snapshot state, so large snapshots can be expensive.
- Submission and advancement across workers do not provide transactional rollback. Integrations that need stronger ordering must supply it at their own boundary.
- The API is still evolving. The root re-exports the current input and application contracts; application-specific macros belong in consumers.

## Verification

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p contime --example counter
cargo run -p contime --features monitor,counters --example client_movement -- --headless --duration 2
cargo test --doc --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Build the Rust API documentation with `cargo doc --workspace --no-deps --open`
for completion boundaries and public contracts. Each crate README describes
its focused tests and benchmarks. Recorded component results stay in those
READMEs; public pipeline and terminal-presentation results are in
[`benches/README.md`](benches/README.md).

The [coverage workflow](.github/workflows/coverage.yml) combines workspace tests
with default and optional features, Manual-only tests, and example verification
tests. It reports library coverage to [Qlty](https://qlty.sh/gh/humacss/projects/contime),
excluding test, benchmark, and example source. Browser and WASI packages are
separate workspaces and are not included in this native coverage report.

## License

MIT
