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
ConTime creates a snapshot when its first event is inserted. `create` initializes
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

The `Example` trait is part of the diagnostic harness, not a ConTime requirement.
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

For a small public-API walkthrough, start with [`main.rs`](main.rs), the program
shown in the root README. It runs two counters for five seconds and queries them
at 20 Hz. Its small random changes and printing are self-contained.

```sh
cargo run --example counter
```

For the configurable live dashboard:

```sh
cargo run --release --features monitor,counters --example counter_live
cargo run --release --features manual,monitor,counters --example counter_live -- --execution manual
```

Choose settings before startup; q, Escape or Ctrl-C stops production and validates
final state. Use the [launcher](../../support/launcher/README.md) for native,
WASI and browser execution, or the [example guide](../../README.md) for common
settings, defaults and measurement semantics. The native composition entry point
is [`support/entrypoints/counter.rs`](../../support/entrypoints/counter.rs).

```sh
cargo run --release --features monitor,counters --example counter_live -- \
  --no-setup --headless --duration 5 --snapshots 1000 --workers 4 \
  --events-per-second 100000 --average-snapshots-per-event 1.1 \
  --max-snapshots-per-event 2 --snapshot-skew 90
```

The [Manual walkthrough](manual.md) demonstrates `process()` without background
threads. The [focused monitor/simulator command](monitored/README.md) uses the
same model and host with a smaller direct command-line interface.

## Check

```sh
cargo test --features manual,monitor,counters --example counter_live --example manual_counter_live
```

Tests cover late arrivals after earlier queries, retained duplicate IDs,
multi-counter events, destination distribution, both execution modes, and
monitor-driven production with final state validation.
