# contime

`contime` is an in-memory Rust crate that builds queryable continuous-time state from unreliable event streams.

`contime` supports bounded memory, enables concurrent multi-threaded/multi-processor processing, designed for very cheap best and average case processing.

## Current State

This crate is currently in development. The API is not stable yet, but the core apply, query, and memory-budget behavior is covered by tests. Documentation, benchmarks, and ergonomics are still being refined.

## Continuous Time?

Most systems today work with **discrete state**: "**right now** the state is X".

`contime` takes a different approach: it builds and maintains **continuous-time state**.

This means that state is defined **at every point in time**, not only at the moments when events arrive, but also in between those events, before those events, and after those events.

Now you can ask the question, what was the state at time `T`? 

This also helps handle out of order events cleanly. What if an event comes in late? 

Many systems will struggle to handle this because all you have is the current state. You don't know what the state looked like at the time the event was supposed to arrive. 

With continuous-time state applying an event from the past is as natural as applying one in the present. This means we get deterministic state at all times, regardless of the order of the events coming in to the system.

## Target Use Cases

`contime` was originally built for the needs of a high-performance authoritative game server, but it is generally useful for any system that requires true continuous-time state and out-of-order event processing.

Typical good fits include:

**Multiplayer game servers and clients**  
Maintain deterministic historical state for delayed, duplicated, or out-of-order player inputs

**IoT & sensor network data**  
Apply delayed, duplicated, or out-of-order readings from unreliable devices and networks into a consistent, queryable timeline

**Audit trails & lightweight event-sourcing**  
Provide time-travel queries and deterministic historical state without needing a full persistent event store

## How it Works

**contime** works by accepting temporal `Input`s and materializing `Snapshot` state from `Event` inputs.

### Snapshots
A `Snapshot` is a discrete state at a particular point in time, for example the state of a particular character in a game at time `T`. The `Snapshot` defines the shape of the data, and the `snapshot_id` in this case defines the character the data belongs to.

### Events
An `Event` is an `Input` that should be applied to one or more `Snapshot`s. `snapshot_id`s can be extracted from the `Event`, and this is then used to apply the event to one particular `Snapshot` and `snapshot_id`.

When an `Event` is applied to a `Snapshot`, the event modifies the `Snapshot` state.

Memory declarations keep retained data and apply-time allocation separate.
`Input::conservative_size` describes the retained event payload.
`Event::conservative_allocation_size` describes additional snapshot-state
memory that applying that event may allocate; it defaults to zero for events
that only mutate inline fields. `Snapshot::conservative_size` describes the
checkpoint itself. `ContimeEvent` accepts `allocation_bytes = ...` when a
derived event allocates during application. ConTime conservatively carries that
allocation through the possible retained checkpoint copies produced by replay.
Markers have no apply allocation.

The system keeps a list of `Checkpoint`s internally to retain previous state, and can generate the state at time `T` by grabbing the closest `Checkpoint` and applying all inputs in order through time `T`. Each checkpoint also retains the cumulative raw history input count through that checkpoint, so replay resumes with both the snapshot state and its deterministic input frontier.

### History storage

Each snapshot history stores canonically ordered arrivals in an array-backed
append deque. Inputs that arrive before the append tail are kept in a separate
B-tree. Replay merges the two already-ordered sources by `(time, input ID)`, so
late events preserve deterministic history without making normal ordered
admission pay for a tree insertion.

Input identity is independent of timestamps while an input is retained.
Each snapshot history owns retained identity and horizon admission. Reusing a
retained ID in that history is therefore an idempotent no-op even if the new
input carries another time or payload. The history's identity index is pruned
with the history horizon, after which the forgotten ID may be admitted again.

`LocalSnapshotHistory::inputs` is a representation-neutral `HistoryInputs`
value. Direct history users should use its ordered iteration, storage-count,
latest-key, insertion, and pruning methods rather than relying on `BTreeMap`
methods or either internal store.

When the retained history horizon advances, ConTime first folds pruned inputs into
the replay-anchor checkpoint and then calls `Snapshot::compact_before` on every
retained checkpoint. The default implementation is a no-op. Snapshots that keep
input IDs or other replay-only references can override the hook, or use the
`compact = { ... }` derive option, to discard references older than the supplied
boundary while preserving their accumulated state.

A checkpoint at time `T` represents the state after the complete input bucket at `T`.
Histories can exist without checkpoints when they contain only markers. Such histories are
pending: their inputs remain retained and replayable, but snapshot queries return `None`
until an event supplies the snapshot identity. Concrete snapshots implement `Default`, and
`SnapshotEvent::set_snapshot_identity` initializes only the identity fields on that clean
default before normal replay begins.

### Flow

The apply path has four explicit boundaries:

```text
API inputs -> snapshot batches -> worker messages -> snapshot histories
```

The API visits each input's `snapshot_id` routes once and groups inputs by
snapshot while preserving first-snapshot and per-snapshot input order. The
`Router` hashes each prepared snapshot ID, partitions complete snapshot batches
into one message per affected worker, and dispatches those messages. It does
not reopen inputs or regroup them.

Each `Worker` maintains a unique set of `snapshot_id`s and works in a dedicated thread running lockless code.

Each worker reserves memory once for its complete message and then passes every
prepared snapshot batch directly to the matching snapshot history. The history
owns duplicate-ID and horizon decisions. A worker responds once after
processing its complete message for a synchronous request. `Contime` owns
request-scoped response channels and merges worker results directly; concurrent
synchronous calls therefore cannot consume one another's responses.

A worker history remains pending while it contains only markers. Its first applicable event
materializes the statically generated snapshot-lane variant inside the history. A snapshot id
must map to exactly one snapshot-lane variant for the lifetime of a `Contime` instance; receiving
an event for a different variant under an already materialized id is an invariant violation and
panics.

The `Worker` applies each snapshot batch to the continuous-time state for its
`snapshot_id` and serves historical queries for that lane.

This design lets `contime` scale across multiple threads and processors with zero lock contention.

## Using `contime`

The public API is small. In practice you do five things:

1. Define a `Snapshot` type and one or more `Event` types.
2. Derive `ContimeEvent` and `ContimeSnapshot`, or implement `Input`, `Event`, `SnapshotEvent`, `Snapshot`, and `ApplyEvents` manually.
3. Generate `SnapshotLanes`, `InputLanes`, and a typed `Contime` alias with `contime::lanes!`.
4. Create a `Contime` instance with a worker count and memory budget.
5. Apply inputs, optionally advance the retained history horizon with `advance_to`, then query state with `query_at`.

`send` is fire-and-forget after worker enqueue and returns
`Result<(), ContimeError>`. `apply` waits only for workers affected by its input
batch and returns `Result<Vec<EventRejection>, ContimeError>`. An empty vector
means every affected worker accepted or idempotently ignored the inputs.
Rejections contain exactly `event_id` and `reason`; the current reasons are
`BeforeHistoryHorizon` and `MemoryFull`. Identical `(event_id, reason)` pairs
from several workers are returned once, while different reasons for one event
remain distinct. `Contime` also owns query ordering, current time, and
advancement completion waits.

Advanced integrations may also add marker variants to the generated
`InputLanes`. Markers are opaque temporal records routed into the same replay
batches as events. An `ApplyWrapper` may interpret the complete input batch and
choose whether markers are forwarded to lane application. Generated default
lanes ignore marker variants.

### Time Types

`contime` does not require timestamps to be integers. Every assembled lane set
uses one type implementing `ContimeTime`, which requires a total order, a
default value, addition and subtraction with itself, and saturating subtraction
for history-horizon calculations. This supports ordered composite times whose
consumers define their own arithmetic semantics without allowing horizon
calculation to overflow.

Manual `Input` and `Snapshot` implementations declare their associated `Time`.
Derives use `time_type`, and the lane manifest declares the same concrete type:

`ContimeSnapshot` reads time from `self.time.clone()` by default. Use the
`time = ...` option only when a snapshot exposes time through another field or
expression.

```rust
#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(
    id = self.id,
    time = self.time.clone(),
    time_type = CompositeTime,
    bytes = 32
)]
struct OrderedEvent {
    id: u128,
    time: CompositeTime,
}

contime::lanes! {
    mod ordered_lanes;
    time CompositeTime;
    snapshots [OrderedSnapshot];
    routes [
        OrderedEvent => [OrderedSnapshot],
    ];
}
```

Inputs are identified by `input_id` and ordered by `(time, input_id)`. A
retained input ID is an idempotent no-op even if it is submitted with another
time. A composite time therefore creates a
separate apply batch for each distinct complete value while retaining normal
out-of-order replay behavior. Horizon advancement subtracts the configured
horizon value using the concrete time type's `ContimeTime::saturating_sub`
implementation.

### Minimal usage flow

The canonical onboarding example now lives in [`examples/ordered_values.rs`](examples/ordered_values.rs).

Run it with:

```bash
cargo run --example ordered_values
```

The example defines one snapshot that stores each received value in event-time order. It then applies a late event on purpose so you can see two things clearly:

- queries at different times return different ordered prefixes of the same history
- re-querying after a late event returns the corrected historical state

The snapshot logic in the example only appends values during replay. The ordered result comes from `contime` replaying events in chronological order, not from custom sorting in the example code.

### Apply Wrappers

`contime` processes one same-complete-time input batch for one snapshot lane at a time.
Advanced callers can provide an `ApplyWrapper` and implement
`apply_input_batch_wrapper` to control how that batch affects the working
snapshot. The default wrapper filters out plain markers and applies the event
subset once. Custom wrappers may call the inner apply one or many times with
temporary input batches. Every wrapper invocation must call the inner apply at
least once; use an empty effective batch when every event is filtered out.
Each inner apply exposes the cumulative raw history input count represented by
the outer replay bucket. If a wrapper partitions one raw bucket into multiple
effective applies, every partition receives the same count.

`reconcile_input_batch_wrapper` is the authoritative-history counterpart. It
is called while accepted inputs reconcile canonical history and delegates to
`apply_input_batch_wrapper` by default. Integrations may override it to publish
effects caused by authoritative history changes. Queries and retention
reconstruction use only `apply_input_batch_wrapper`, so they reconstruct the
same snapshot state without republishing those effects.

`ApplyInner` owns the mutable snapshot reference while the wrapper runs.
Wrappers may inspect the resulting snapshot through its immutable `snapshot`
accessor, but they cannot mutate snapshot state directly. Wrappers are
infallible and every remaining input batch is replayed after the wrapper
returns.

### History Input Counts

A history input count is the cumulative number of unique raw inputs represented
by one snapshot history through the end of a complete-time bucket. It includes
events and markers before an `ApplyWrapper` filters or partitions the bucket,
including inputs compacted into a retained horizon checkpoint.

```text
next_count = checkpoint_count + raw_bucket_input_count
```

Occupied retained input IDs are idempotent no-ops and do not increase the
count. Marker-only corrections increase it even when the effective event set
is empty. Late inputs replay the affected suffix and increase every later
frontier deterministically. The resulting count is available as
`ApplyInner::history_input_count` and, when concrete snapshot application runs,
as `ApplyBatch::history_input_count`. Checkpoints store the same count with the
snapshot and restore both before replaying a later suffix.

### Markers

A marker has a canonical id and complete ordered time, like an event, but has
no default snapshot behavior. One marker is globally identified across the
ConTime instance even when its routing makes it visible to several snapshot
histories. An apply wrapper may forward markers to custom lane application;
generated default lanes ignore them.

Applying a marker with a new `(time, id)` replays each affected history from
the marker's time. Applying an occupied identity again is an idempotent no-op;
ConTime does not compare or replace input payloads. The
default apply wrapper ignores markers, while custom wrappers receive events and
markers together in an `InputBatch` and define all marker semantics.

Markers may create a pending routed history, but they cannot initialize snapshot
state. A marker-only history therefore does not invoke an apply wrapper or
materialize a query result until an event supplies the snapshot identity.
Retained markers follow the same per-snapshot history horizon as events.

## `contime` is

- A continuous-time state engine  
- Handles out-of-order and duplicate events  
- Designed for very fast operation on one server using all available processors  
- Is in memory only and designed to never OOM with bounded memory

In short: a high-performance, in-memory, continuous-time state engine.

## `contime` is not

- A persistent database or storage layer  
- A general-purpose stream processor  
- A full event-sourcing system

`contime` is very specialized for one particular use-case. 

It maintains queryable historical state from applied events.

## Current Constraints

- `contime` is in-memory only. Persistence, replay from disk, and recovery belong in surrounding systems.
- Memory usage is bounded by the configured budget and conservative consumer declarations.
- History pruning is driven by `advance_to` together with the configured history horizon.
- Snapshot queries include events with `event.time() <= query_time`.
- Checkpoints currently clone full snapshots, so the crate is best suited to relatively small snapshot payloads today.
- Concurrent event application and horizon advancement are not transactionally
  ordered across workers. Callers requiring all-or-nothing behavior must
  serialize application and advancement.

### Provisional memory consistency

Memory admission is intentionally conservative and not yet transactional:

- The API estimates retained events, declared apply allocations across possible
  checkpoint copies, identity bookkeeping, clean snapshot materialization, and
  complete checkpoint storage before dispatch. This check is advisory;
  concurrent requests can both pass it.
- Each worker atomically reserves its complete message before mutating any of
  that message's snapshot histories. Existing replay-checkpoint space is added
  to the worker reservation.
- Separate workers reserve independently. One worker can therefore apply an
  event while another rejects its message, and a multi-snapshot request can be
  partially applied when its snapshots route to different workers.
- Synchronous `apply` reports the IDs and reason codes rejected by affected
  workers. Asynchronous `send` is best effort after enqueue and has no later
  rejection result.
- Conservative estimates may reject work that would have fit. Understating
  `conservative_size`, `conservative_allocation_size`, or a derived event's
  `allocation_bytes` violates the memory-accounting contract.

Cross-worker transactional reservation and rollback are deferred.

## Current Status

The crate is currently a work in progress. The API is not stable yet and there are still some notable gaps:

- Crate wide benchmarks
- Early exits on apply
- More examples and deeper documentation for multi-snapshot setups
- Clones snapshots for checkpoints. This is fine for small snapshots <1KB. For supporting larger snapshots we need deltas.

## Performance snapshot

These Criterion measurements were collected on 2026-08-26 on an Apple M3 Pro,
macOS 26.3.1 (25D771280a), with rustc 1.90.0 and the optimized benchmark
profile. Every interval is Criterion's exact `[low estimate high]` result.

### Apply pipeline, outside in

Every row applies the same 1,000 unique events to one new snapshot. Fixture
construction and worker startup/warm-up happen outside the timed region. The
API row begins with already-built inputs because API grouping is part of that
boundary; the lower rows begin with their already-prepared boundary input.
Each row removes exactly one outer subsystem:

- API is the complete synchronous `Contime::apply` round trip.
- Router receives prepared snapshot batches, partitions and dispatches them,
  and waits for the affected worker.
- Worker receives one already-partitioned snapshot message, reserves it once,
  applies its snapshot batch, and replies.
- Snapshot history applies the per-snapshot input batch directly without a
  worker, channel, router, or API.

| Measured entry point | Total time | Time per event | Approximate cost added over the next row |
| --- | ---: | ---: | ---: |
| Public API | `[64.703 µs 65.048 µs 65.530 µs]` | `65.048 ns` | `~9.625 µs` grouping, API completion, and result merge |
| Router | `[55.322 µs 55.423 µs 55.529 µs]` | `55.423 ns` | not separable from worker at this resolution |
| Worker | `[55.087 µs 55.247 µs 55.437 µs]` | `55.247 ns` | `~13.357 µs` worker message, reservation, lookup, and dispatch |
| Snapshot history | `[41.696 µs 41.890 µs 42.173 µs]` | `41.890 ns` | direct history baseline |

The approximate costs subtract Criterion point estimates. They are diagnostic,
not independent measurements. Router and worker confidence intervals overlap,
so their `176 ns` point-estimate difference is not separable from scheduling
noise. The largest remaining outer-layer residual is the `~13.357 µs` between
worker entry and direct history entry; the history itself remains the dominant
part of the full `65.048 µs` API round trip.

Reproduce the 30-sample stack with:

```bash
cargo test --test apply_boundary_benchmarks
cargo bench --bench apply_boundaries -- apply_1000_events_one_snapshot --sample-size 30
```

### Hybrid-history workloads

Each late-rate case applies 1,000 new inputs after an established ordered tail.
Merged replay reconstructs 1,000 inputs after that tail from the two sorted
stores. Horizon pruning advances to a drop boundary of `500`; ordered and mixed
fixtures remove 500 inputs, while the late-only fixture removes 500 late-tree
inputs and retains its one ordered tail sentinel.

| Workload | Shape | Time |
| --- | --- | ---: |
| Insert and reconcile 1,000 inputs | 0% late | `[43.243 µs 43.345 µs 43.408 µs]` |
| Insert and reconcile 1,000 inputs | 1% late | `[57.002 µs 57.115 µs 57.203 µs]` |
| Insert and reconcile 1,000 inputs | 10% late | `[73.544 µs 73.833 µs 74.089 µs]` |
| Insert and reconcile 1,000 inputs | 50% late | `[80.871 µs 82.473 µs 85.870 µs]` |
| Insert and reconcile 1,000 inputs | fully late, reverse-ordered batch | `[85.356 µs 86.633 µs 88.151 µs]` |
| Replay 1,000 merged inputs | 0% late-tree density | `[6.0963 µs 6.1214 µs 6.1449 µs]` |
| Replay 1,000 merged inputs | 10% late-tree density | `[5.9662 µs 5.9956 µs 6.0129 µs]` |
| Replay 1,000 merged inputs | 50% late-tree density | `[3.8389 µs 3.9031 µs 3.9426 µs]` |
| Advance horizon and prune | ordered only | `[4.6183 µs 4.6750 µs 4.7083 µs]` |
| Advance horizon and prune | late only | `[12.625 µs 12.791 µs 12.882 µs]` |
| Advance horizon and prune | mixed | `[4.5880 µs 4.6614 µs 4.7208 µs]` |

Reproduce the focused matrix with:

```bash
cargo bench --bench apply --no-run
cargo bench --bench apply -- history_late_rate --sample-size 20
cargo bench --bench apply -- history_reverse_batch --sample-size 20
cargo bench --bench apply -- history_merged_replay --sample-size 20
cargo bench --bench apply -- history_horizon_prune --sample-size 20
```

These measurements do not include outer Timeless Runtime orchestration.

## TODO / Future Improvements

- Builder-style configuration instead of positional constructor arguments
- Compiled examples for more complex multi-snapshot topologies
- Delta-based checkpoints for larger snapshots
- Refreshed crate-wide benchmarks and performance guidance

## Real-World Usage

Currently used in Arcanex, a tickless event-driven multiplayer game engine, where all event streams are consumed using `contime`.

## License
MIT
