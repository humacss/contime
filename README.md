# contime

`contime` is an in-memory Rust workspace that builds queryable continuous-time state from unreliable event streams.

It handles delayed, duplicated, and out-of-order events, and distributes independent snapshot histories across workers. The root `contime` crate assembles the running engine. [`contime-core`](https://github.com/humacss/contime/blob/main/crates/core/README.md) connects its protocol and storage contracts; the other crates provide the individual parts.

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

## How it Works

An event describes something that happened at a particular time. A snapshot describes the state of one logical entity, identified by a snapshot ID. An input can route an event to one or more snapshot histories.

Each history keeps canonically ordered events and materialized checkpoints. To answer a query at time `T`, ConTime reconstructs state from a checkpoint and applies the events through `T`, including events exactly at `T`. Late events invalidate the affected checkpoint suffix and replay it in the correct order. Duplicate retained event IDs do not apply twice within a history.

Checkpoints are replay optimizations, not a separate source of truth. Application hooks receive complete timestamp batches. Snapshot queries reconstruct state without publishing application effects or advancing processing.

The apply path has explicit boundaries:

```text
inputs -> API batching -> admission coordinator -> routers
       -> worker histories -> checkpoint replay -> snapshot application
```

Routers share immutable events across their destination histories. Each worker owns its mutable histories and serves their queries. The worker processes replay in bounded steps so that busy histories and query traffic can both make progress. Consumers can connect application hooks and forward derived inputs through the same pipeline.

Processing and retention are separate. `advance_to` moves the processing target forward. Future events can be queried before that target reaches them, while application effects wait for processing. Coordinated progress determines when old history can safely be pruned; a requested advancement is not evidence that every worker has finished it. Pruning retains the state needed to replay the remaining history.

## Using `contime`

Depend on `contime` for the running pipeline. The root owns configuration, startup, submission, queries, progress subscriptions, idle checks, and shutdown. Core retains admission rules and the adapters connecting the individual subsystems.

1. Define your time type, event inputs, and snapshot state.
2. Implement Core's input routing and checkpoint application contracts.
3. Start `ConTime` with router, worker, checkpoint, and retention settings.
4. Submit inputs, advance processing as needed, and query historical state.
5. Shut down the owned topology when finished.

The [`ordered_values` example](https://github.com/humacss/contime/blob/main/examples/ordered_values.rs) shows how a late event changes historical queries without custom sorting:

```bash
cargo run -p contime --example ordered_values
```

For example, after defining the event and snapshot contracts, a late input can
change historical results without changing your application loop:

```rust,ignore
let history = ConTime::<ReceiveValue, OrderedValues, ()>::start(config, ())?;
history.advance_to(Time(10))?;
history.apply([input(100, 5, 50), input(101, 10, 100)])?;
history.wait_until_idle(Duration::from_secs(2))?;
assert_eq!(history.query_at(Time(11), [1])?[0].values, [50, 100]);

history.apply([input(102, 7, 70)])?;
history.wait_until_idle(Duration::from_secs(2))?;
assert_eq!(history.query_at(Time(8), [1])?[0].values, [50, 70]);
assert_eq!(history.query_at(Time(11), [1])?[0].values, [50, 70, 100]);
history.shutdown();
```

This excerpt uses the types and configuration from the complete
[ordered-values example](https://github.com/humacss/contime/blob/main/examples/ordered_values.rs). It also shows why an
explicit idle wait belongs before assertions about completed processing; a
query alone is not a processing barrier. The example additionally submits an
identical retained event twice to demonstrate deduplication.

The [sensor-readings example](https://github.com/humacss/contime/blob/main/examples/sensor_readings.rs) uses request-scoped
insertion acknowledgements, historical snapshot queries, and half-open event
ranges. Both examples use only public ConTime APIs and ordinary application data.

```bash
cargo run -p contime --example sensor_readings
```

ConTime is not published on crates.io yet. For local development, depend on a
checkout with `contime = { path = "/path/to/contime" }`. The workspace packages
keep publishing disabled until a release is explicitly prepared.

ConTime provides asynchronous submission and response receivers alongside convenience query methods. Successful submission means that work was queued for admission. Request-scoped completion channels, idle waits, replay listeners, and progress observations provide different completion boundaries; choose the one your integration needs. Rejections are reported separately from submission failures.

Snapshot replay listeners observe relevant replay. `subscribe_pruned_horizon` reports completed pruning horizons. `ProgressPolicy` receives local processing observations; these observations do not establish delivery or durable persistence in an external system.

Applications supply their own event types, snapshot state, and deterministic application logic. Serialization, persistence, and network transport remain outside ConTime.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `contime` (root) | Public engine handle, configuration, startup, operations, and shutdown |
| [`contime-core`](https://github.com/humacss/contime/blob/main/crates/core/README.md) | Admission protocol, messages, and router/worker storage adapters |
| [`contime-api`](https://github.com/humacss/contime/blob/main/crates/api/README.md) | Input batching and request boundaries |
| [`contime-router`](https://github.com/humacss/contime/blob/main/crates/router/README.md) | Deterministic routing to worker-owned histories |
| [`contime-runtime`](https://github.com/humacss/contime/blob/main/crates/runtime/README.md) | Thread topology, channels, and shutdown |
| [`contime-worker`](https://github.com/humacss/contime/blob/main/crates/worker/README.md) | History ownership and incremental replay scheduling |
| [`contime-snapshots`](https://github.com/humacss/contime/blob/main/crates/snapshots/README.md) | Checkpoint storage and historical reconstruction |
| [`contime-events`](https://github.com/humacss/contime/blob/main/crates/events/README.md) | Ordered retained event history |
| [`contime-lanes`](https://github.com/humacss/contime/blob/main/crates/lanes/README.md) | Consumer application and hook contracts |
| [`contime-progress`](https://github.com/humacss/contime/blob/main/crates/progress/README.md) | Participant progress measurement |

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
cargo run -p contime --example ordered_values
cargo run -p contime --example sensor_readings
cargo test --doc --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Build the Rust API documentation with `cargo doc --workspace --no-deps --open`
for completion boundaries and public contracts. Each crate README describes
its focused tests and benchmarks.

## License

MIT
