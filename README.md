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

## Using `contime`

Depend on `contime` for the running pipeline. The root owns configuration, startup, submission, queries, progress subscriptions, and shutdown. Core retains admission rules and the adapters connecting the individual subsystems.

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
let counter = 7;
insert(&history, [input(counter, 5, 50), input(counter, 10, 100)])?;
assert_eq!(history.query_at(Time(11), [values_id(counter)])?[0].values, [50, 100]);

insert(&history, [input(counter, 7, 70)])?;
assert_eq!(history.query_at(Time(8), [values_id(counter)])?[0].values, [50, 70]);
assert_eq!(history.query_at(Time(11), [values_id(counter)])?[0].values, [50, 70, 100]);
history.shutdown();
```

This excerpt uses the types and configuration from the complete
[ordered-values example](https://github.com/humacss/contime/blob/main/examples/ordered_values.rs). It also shows why a
request-scoped insertion acknowledgement belongs before querying submitted
data. Its `insert` helper waits for insertion responses; it does not wait for
global idle or completed live replay. The example additionally submits an
identical retained event twice to demonstrate deduplication.

Both examples use [`contentid`](https://github.com/humacss/contentid) to derive local event IDs from the complete event through Rust's `Hash` trait, without binary serialization. Snapshot IDs combine a snapshot type identifier with the fields identifying its history, such as a counter or sensor key. Routing and queries use the same calculation; changing snapshot state does not change that history's ID. Composite history keys can contain several fields.

Identical events have the same ID. If otherwise identical events represent separate occurrences, include a distinguishing field such as a sequence number in the event. These examples use local IDs for one running application, widening the 64-bit hashes to ConTime's `u128` ID type. Applications that exchange IDs across systems can instead use distributed IDs.

`contentid` is a development dependency for these examples; ConTime's library does not depend on it. Until `contentid` is published on crates.io, the examples use a pinned revision from its public Git repository.

The [sensor-readings example](https://github.com/humacss/contime/blob/main/examples/sensor_readings.rs) uses request-scoped
insertion acknowledgements, historical snapshot queries, and half-open event
ranges. Both examples use only public ConTime APIs and ordinary application data.

```bash
cargo run -p contime --example sensor_readings
```

ConTime is not published on crates.io yet. For local development, depend on a
checkout with `contime = { path = "/path/to/contime" }`. The workspace packages
keep publishing disabled until a release is explicitly prepared.

ConTime provides asynchronous submission and response receivers alongside convenience query methods. Successful submission means that work was queued for admission. Request-scoped completion channels, replay listeners, and progress observations provide different completion boundaries; choose the one your integration needs. Idle waits are helpers for tests and benchmarks, not synchronization for running applications. Rejections are reported separately from submission failures.

Snapshot replay listeners observe relevant replay. `subscribe_pruned_horizon` reports completed pruning horizons. `ProgressPolicy` receives local processing observations; these observations do not establish delivery or durable persistence in an external system.

Applications supply their own event types, snapshot state, and deterministic application logic. Serialization, persistence, and network transport remain outside ConTime.

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
