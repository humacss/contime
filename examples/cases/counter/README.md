# Counter

This is the smallest live case: an event adds a number to one or more counters.
Its application logic is deliberately cheap, making it useful for exploring
throughput without hashing or a more involved state update. The measured live
rate still includes generation, routing, storage, monitoring and host overhead.
Use [chain hash](../chain_hash/README.md) to verify ordering and
[client movement](../client_movement/README.md) for a practical state model.

## Events and snapshots

Read [`model.rs`](model.rs) first. It defines one event type, `CounterChanged`,
and one snapshot type, `Counter`. Different counter IDs select independent
snapshots of that same type; there is no separate Rust type per counter.

| Event field | Meaning |
| --- | --- |
| `id` | Stable event identity; a retained retry reuses it |
| `time` | When the change applies, independently of when it arrives |
| `targets` | Distinct counter snapshot IDs to update |
| `value` | Amount to **add**, rather than a replacement total |

For example, a change with `targets = [1, 2]` and `value = 1` increments both
counters once. It is one source event and two snapshot applications. Their
updates are independent, including when the snapshots belong to different workers.

A `Counter` stores its identity (`id`), state time (`time`), accumulated total
(`value`), and number of represented event applications (`events`). The last
field helps validate the result; it is not a count of worker replay attempts.
`contime` creates a snapshot when its first event is inserted. `create` initializes
it at zero; `apply_events` applies the first event and subsequent events.

## Connecting the lane

Three application contracts define the relationship:

1. `Event` returns the change's timestamp.
2. `Input` returns its event ID and emits the destination counter IDs.
3. `ApplyEvents<CounterChanged> for Counter` adds each change to a counter.

[`setup.rs`](setup.rs) selects `CounterChanged` and `Counter` for the shared
example tools through `CounterExample`. The environment uses those types to
construct `ConTime<CounterChanged, Counter, ()>` or its Manual equivalent.
There is no additional lane-registration step in this example.

The `Example` trait is part of the diagnostic harness, not a `contime` requirement.
Its `record` hook builds separate expected state for validation; its inspection
hooks let shared presentation read IDs, totals and counts. Runtime startup,
Manual processing, monitoring and UI remain outside the case definition.

## Generating traffic

[`monitored/generation.rs`](monitored/generation.rs) supplies the simulator's two
callbacks: create an increment event, then assign its destination counter IDs.
Live events add one and receive increasing event IDs. The configured snapshot
population, average/maximum destinations and traffic skew determine where they go.
The generator knows nothing about threads, browser frames or terminal rendering.

Maximum production uses worker-monitor feedback independently of snapshot
queries. Counters are queried and checked against the generated reference at
shutdown. Addition is order-insensitive, so matching counter totals does not
establish correct event ordering; the chain-hash case checks that separately.

## Run

For a small public-API walkthrough, start with [`readme.rs`](../../readme.rs), the program
included in the Rust API documentation. It inserts ordered-list events individually, checks late replay, and verifies
that retrying an event does not apply it twice.

```sh
cargo run --example readme --features macros
```

The counter workload is retained as an internal test and benchmark fixture.
The public launcher offers the Game and Chain hash examples.

The [Manual walkthrough](manual.md) demonstrates `process()` without background
threads. See the [simulator callbacks](monitored/README.md) for the internal workload.

## Check

```sh
cargo test --all-features --test counter_internal
```

Tests cover late arrivals after earlier queries, retained duplicate IDs,
multi-counter events, destination distribution, both execution modes, and
monitor-driven production with final state validation.
