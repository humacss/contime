![ConTime — continuous-time state from unreliable event streams](assets/contime-header.svg)

[![Crates.io](https://img.shields.io/crates/v/contime.svg)](https://crates.io/crates/contime)
[![Docs](https://docs.rs/contime/badge.svg)](https://docs.rs/contime)
[![Qlty | Maintainability](https://qlty.sh/gh/humacss/projects/contime/maintainability.svg)](https://qlty.sh/gh/humacss/projects/contime)
[![Qlty | Code Coverage](https://qlty.sh/gh/humacss/projects/contime/coverage.svg)](https://qlty.sh/gh/humacss/projects/contime)

> **Work in progress:** ConTime is still being developed for my own use. There is more cleanup to do, and the API needs further refinement before it is ready for general adoption. You are welcome to try it, but expect changes and rough edges.

ConTime is a Rust library for building state from events, even when those events
arrive late or out of order. You define the events and how they change your state;
ConTime keeps the history and replays the affected state when earlier input arrives.

## Reconciliation in action

[![The same delayed inputs with ConTime reconciliation on the left and without reconciliation on the right.](assets/reconciliation.gif)][browser-demo]

A delayed input can leave a character in the wrong place. If you apply each
movement change only when it arrives, those errors accumulate.

Both sides of this demo receive the same inputs. **On the left, ConTime corrects
the past when a late input arrives**, bringing the character back onto its intended
path. **On the right, without reconciliation, the route drifts.**

ConTime handles the history and replay behind those corrections, so you can build
reconciliation into your application without implementing it yourself.

**[Play the demo][browser-demo]** ·
[Read about the example](examples/cases/client_movement/README.md) ·
[View the code](examples/cases/client_movement/model.rs)

## Performance

On the author's Apple M3 Pro, the public API benchmark processed **about 5.2 million
simple counter events per second with one worker**, submitting events individually.
This includes submission, routing and processing, excluding event construction and
runtime startup.

A separate order-sensitive hash workload, with 64 snapshot histories and batches
of 1,000 events, recorded:

| Worker threads | Total throughput |
| ---: | ---: |
| 1 | 4.0 million events/s |
| 2 | 8.0 million events/s |
| 4 | 17.9 million events/s |

These release-mode results were recorded on 2026-10-28 with Rust 1.90.0, unpinned
threads and normal desktop background activity. Your event logic, fanout, late
arrivals and queries affect throughput. See the [pipeline benchmarks](benches/README.md)
and [scaling results](benches/scaling/README.md) for the measured workloads and commands.

## Continuous time

ConTime lets you ask **“what was the state at time T?”** within retained history,
including times between events. An event's timestamp determines where it belongs
in that history; its arrival time does not. A late event can therefore change
what a later query returns.

An **event** describes a timestamped change. A **snapshot** represents the state
of one entity, such as a client or a list. ConTime maintains that entity's history
so it can reconstruct state at the time you request. It keeps history in memory;
your application owns persistence and chooses how much history to retain.

## Use cases

- **Multiplayer games:** reconstruct state from delayed or out-of-order player inputs.
- **Sensors and devices:** combine readings that arrive over unreliable networks.
- **Historical application state:** query past state without building replay yourself.

## Using `contime`

ConTime 0.1.2 requires Rust 1.90 or newer. Runtime worker threads are enabled by
default; enable the optional macros for the helpers shown here:

```toml
[dependencies]
contime = { version = "0.1.2", features = ["macros"] }
```

The [complete ordered-list example](examples/readme.rs) defines a `Value` event
with an ID, timestamp, target list and character, and a `List` snapshot that stores
those characters. Derives handle identity, time and event registration. The
application rule and module are:

```rust
use contime::prelude::*;

impl SnapshotApply for List {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Value>() {
            self.values.push(event.value);
        }
    }
}

#[derive(ContimeModule)]
#[contime(snapshots = [List])]
struct Lists;
```

Start ConTime and submit values in the order **c, a, b**. Their timestamps put
them in **a, b, c** order:

```rust
let contime = ConTime::<Lists>::start(configuration())?;
contime.apply([
    Value { event_id: 1, list_id: 7, time: 30, value: 'c' }.into(),
    Value { event_id: 2, list_id: 7, time: -10, value: 'a' }.into(),
    Value { event_id: 3, list_id: 7, time: 10, value: 'b' }.into(),
]).map_err(api_error)?;
contime.advance_to(30).map_err(api_error)?;
contime.wait_until_idle(std::time::Duration::from_secs(5))?;

let snapshots = contime.query_at(30, [7]).map_err(api_error)?;
let ListsSnapshotLanes::List(list) = &*snapshots[0];
assert_eq!(list.values, ['a', 'b', 'c']);
```

These are excerpts: the [full source](examples/readme.rs) includes the data
definitions, configuration, error handling and shutdown. It submits each event
separately and checks the result after each insertion, then retries an event to
show that duplicate IDs are ignored. Its idle waits make the assertions easy to
follow; live applications use operation-specific responses or progress.

```sh
cargo run --example readme --features macros
```

## Beyond games

A delayed service-health report can reveal an outage after recovery has already
arrived. The [service uptime example](examples/cases/service_uptime/README.md)
shows ConTime correcting that history and its downtime totals. Try it alongside
client movement in the example launcher.

## Where to go next

| I want to… | Read |
| --- | --- |
| Define events, snapshots, IDs and composable modules | [Application and macro guide](guides/modules/README.md) |
| Configure ConTime or drive Manual processing from my own loop | [Usage guide](guides/usage/README.md) |
| Understand queries, replay, checkpoints and retention | [How ConTime works](guides/how-it-works/README.md) |
| Try the live examples or run them in WebAssembly | [Examples](examples/README.md) |
| Explore throughput and reproduce measurements | [Benchmarks](benches/README.md) |
| Find API details | [Rust documentation](https://docs.rs/contime) |
| Understand the codebase and current limitations | [Workspace guide](guides/workspace/README.md) |

[MIT license](LICENSE)

[browser-demo]: https://humacss.github.io/contime/?domain=client-movement&clients=5&client_spacing=16&movement_speed=12&input_hz=1&query_hz=20&average_latency_ms=500&jitter_ms=500&trail_ms=5000&fade_ms=5000&retention_ms=10000&checkpoint_interval=128&max_ms=2&max_steps=1000&autostart=1
