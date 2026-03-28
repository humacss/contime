# contime

`contime` is an in-memory Rust crate that builds queryable continuous-time state from unreliable event streams.

`contime` supports bounded memory, enables concurrent multi-threaded/multi-processor processing, designed for very cheap best and average case processing.

## Current State

This crate is currently in development. The API is not stable yet, but the core apply, query, reconciliation, memory-budget, and handle-based behavior is covered by tests. Documentation, benchmarks, and ergonomics are still being refined.

## Continuous Time?

Most systems today work with **discrete state**: "**right now** the state is X".

`contime` takes a different approach: it builds and maintains **continuous-time state**.

This means that state is defined **at every point in time**, not only at the moments when events arrive, but also in between those events, before those events, and after those events.

Now you can ask the question, what was the state at time `T`? 

This also helps handle out of order events cleanly. What if an event comes in late? 

Many systems will struggle to handle this because now you need to reconcile this event with already applied state, and all you have is the current state. You don't know what the state looked like at the time the event was supposed to arrive. 

With continuous-time state applying an event from the past is as natural as applying one in the present. This means we get deterministic state at all times, regardless of the order of the events coming in to the system.

## Target Use Cases

`contime` was originally built for the needs of a high-performance authoritative game server, but it is generally useful for any system that requires true continuous-time state and out-of-order event processing.

Typical good fits include:

**Multiplayer game servers and clients**  
Handle unreliable networks, lag compensation, client-side prediction, anti-cheat reconciliation, and perfect replays

**IoT & sensor network data**  
Reconcile delayed, duplicated, or out-of-order readings from unreliable devices and networks into a consistent, queryable timeline

**Audit trails & lightweight event-sourcing**  
Provide time-travel queries and deterministic historical state without needing a full persistent event store

## How it Works

**contime** works by accepting `Snapshot` and `Event` inputs from the user. 

### Snapshots
A `Snapshot` is a discrete state at a particular point in time, for example the state of a particular character in a game at time `T`. The `Snapshot` defines the shape of the data, and the `snapshot_id` in this case defines the character the data belongs to.

### Events
An `Event` is data that should be applied to one or more `Snapshot`s. `snapshot_id`s can be extracted from the `Event`, and this is then used to apply the event to one particular `Snapshot` and `snapshot_id`. 

When an `Event` is applied to a `Snapshot`, the event modifies the `Snapshot` state. 

The system keeps a list of `Checkpoint`s internally to keep previous state, and can generate the state at time `T` by grabbing the closest `Checkpoint`, and applying all events in order up until time `T`.

### Flow

When an `Event` or `Snapshot` input arrives from the user, it is sent to the `Router`.  

The `Router` extracts the `snapshot_id` from the input, hashes it, and routes everything to the correct `Worker`.

Each `Worker` maintains a unique set of `snapshot_id`s and works in a dedicated thread running lockless code.

The `Worker` applies the input to the continuous time state for that `snapshot_id`, and then notifies the user when state changes and reconciliation is needed.

This design lets `contime` scale across multiple threads and processors with zero lock contention.

## Using `contime`

The public API is small. In practice you do five things:

1. Define a `Snapshot` type and one or more `Event` types.
2. Implement `ApplyEvent` so events can mutate snapshots.
3. Generate `SnapshotLanes`, `EventLanes`, and a typed `Contime` alias with `contime::contime!`.
4. Create a `Contime` instance with a worker count and memory budget.
5. Apply events or snapshots, then query state and react to reconciliation notifications.

### Minimal usage flow

The canonical onboarding example now lives in [`examples/ordered_values.rs`](examples/ordered_values.rs).

Run it with:

```bash
cargo run --example ordered_values
```

The example defines one snapshot that stores each received value in event-time order. It then applies a late event on purpose so you can see two things clearly:

- queries at different times return different ordered prefixes of the same history
- a previously queried receiver gets a `Reconciliation` when late data changes historical state

The snapshot logic in the example only appends values during replay. The ordered result comes from `contime` replaying events in chronological order, not from custom sorting in the example code.

If you prefer handle-based integration instead of blocking calls, use `send_event`, `send_snapshot`, `query_at`, and `send_advance`, then wait, poll, or `await` the returned handles.

## `contime` is

- A continuous-time state reconciliation engine  
- Handles out-of-order and duplicate events  
- Designed for very fast operation on one server using all available processors  
- Is in memory only and designed to never OOM with bounded memory

In short: a high-performance, in-memory, continuous-time reconciliation engine.

## `contime` is not

- A persistent database or storage layer  
- A general-purpose stream processor  
- A real-time scheduler  
- A full event-sourcing system

`contime` is very specialized for one particular use-case. 

It sits in between other systems like persistence, scheduling, and event sourcing. It does not attempt to be a general solution for event processing. 

## Current Constraints

- `contime` is in-memory only. Persistence, replay from disk, and recovery belong in surrounding systems.
- Memory usage is bounded by the configured budget and the amount of retained history.
- History pruning is driven by `advance` together with the configured history horizon.
- Queries currently include events with `event.time() < query_time`, not events exactly at `query_time`.
- Checkpoints currently clone full snapshots, so the crate is best suited to relatively small snapshot payloads today.

## Current Status

The crate is currently a work in progress. The API is not stable yet and there are still some notable gaps:

- Crate wide benchmarks
- Early exits on apply
- More examples and deeper documentation for multi-snapshot setups
- Clones snapshots for checkpoints. This is fine for small snapshots <1KB. For supporting larger snapshots we need deltas.

The current benchmarks are not up-to-date and accurate with the code changes over the last few weeks. 

## TODO / Future Improvements

- Builder-style configuration instead of positional constructor arguments
- More ergonomic reconciliation subscription and query helpers
- Compiled examples for more complex multi-snapshot topologies
- Delta-based checkpoints for larger snapshots
- Refreshed crate-wide benchmarks and performance guidance

## Real-World Usage

Currently used in Arcanex, a multiplayer game engine, where all event streams are consumed using `contime`, with state derived from timestamped events and support for prediction, reconciliation, and querying across time.

## License
MIT
