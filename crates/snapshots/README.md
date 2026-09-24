# contime-snapshots

Snapshot storage and playback, independent of event storage, workers, routers,
and runtime effects. The crate has no production dependencies.

| File | Responsibility |
| --- | --- |
| api.rs | Public worker-facing ownership, insert, query, process-until, forward, and prune methods. |
| apply.rs | Apply timestamp batches; update checkpoint time and event count. |
| replay.rs | Replay through an inclusive target, retaining rebuilt checkpoints. |
| query.rs | Reconstruct fresh state without retaining it. |
| forward.rs | Retain state immediately before a new horizon; leave physical cleanup to the consumer. |
| store.rs | Own history, checkpoint spacing, and validity boundaries. |
| playback.rs | Hold a working clone and lend borrowed event intervals. |
| commit.rs | Restricted retention operations and interval progress. |
| types.rs | Checkpoint state and consumer contracts for snapshots, events, storage, and application. |

## Contracts

Construct `SnapshotStore::new(events, snapshot, checkpoint_interval)`. It owns a private
Store and exposes `insert(event)`, `query(time, context)`, `process_until(time, context)`,
`forward(horizon, context, hook)`, and `prune()`. Workers do not access Store, Playback, or
Commit directly. These methods delegate to the internal operations below;
`insert()` delegates admission and invalidation to Store. `prune()` delegates to
Store to remove events before its current horizon and obsolete checkpoints,
preserving the horizon's predecessor checkpoint. It does not apply events,
invoke hooks, or change dirty or horizon, and repeated calls are harmless.
`earliest_replay_time()` reads the first event after the valid replay checkpoint,
including any unchanged prefix invalidated by a late insertion. It does not clone
or apply snapshots. Coordinators can use this bound for publication safety; it is
not interchangeable with the inserted event's timestamp. None means no pending
event application. Completed events at the horizon do not pin future-only work.
The initial snapshot represents state before the supplied history; a zero event
count distinguishes it from a completed first timestamp. Interval zero means
unbounded; other intervals are event counts, extended to finish a whole timestamp.

Implement `EventStore` to supply canonically ordered borrowed iteration.
Event identity and ordering within a timestamp belong to that implementation.
Implement `EventStore::prune_before` to remove events strictly before its supplied
horizon without changing the ordering or identity of retained events.
Implement the optional `InsertEventStore` trait to enable insertion. Its `insert`
method returns `Insert::Inserted`, `Duplicate`, or `BeforeHorizon`; only `Inserted`
may change history. Canonical ordering and event identity remain adapter-owned.
Store rejects timestamps before its horizon before calling the adapter, even when
physical pruning has been deferred. Successful insertion invalidates checkpoints
at that timestamp and afterward, without applying any events. Duplicate/rejected
insertion does not change checkpoint validity. An event at the horizon is accepted.

Validity rolls back to an existing checkpoint strictly before the inserted event,
without moving an already-earlier valid boundary forward. For insertion at the
initial snapshot timestamp (including the minimum timestamp), Store keeps the
original pre-event checkpoint and discards stale same-time/future checkpoint slots.
This avoids requiring timestamp subtraction for insertion. The application and
playback kernels do not need insertion-specific logic. Queries reconstruct fresh
state after insertion; `process_until` retains that reconstructed progress.

Implement `Apply<Checkpoint<S>, Context>` for events. Application receives an
iterator and shared context. The kernel drains unread events and records the
complete timestamp and count after successful application. Use an effect-free
context for queries; application effects are consumer-owned.

## Worker/core integration boundary

The worker can insert multiple events before calling `process_until(target, context)`;
insertion never executes application callbacks. `Inserted` schedules work, while
`Duplicate` and `BeforeHorizon` leave any existing pending work unchanged. The target
is inclusive. Core supplies the event-store adapter and maps its admission outcomes
to this crate's independent `Insert` enum; snapshots does not depend on the events crate.

Core also creates the initial snapshot and supplies application context and the
forwarding hook. Query application must suppress external effects. Forwarding and
physical pruning are separate calls, so pruning completion can be reported only
after both finish for the relevant histories.

`events()` provides read-only access to the consumer-owned event store for its
event-query API; mutation still goes through `insert` and `prune`. Core uses this
accessor for owned event-query results. Scheduling remains worker-owned: the worker
tracks pending boundaries and accepted event timestamps, so no next-event-time
accessor is needed here. `process_until` does not yield between timestamps or
enforce a worker's global advancement limit. Workers choose the target and return
to message handling between calls. Core now connects these APIs to its message worker.

## Internal playback

`Store::play(start)` clones the latest valid checkpoint at or before the
requested time. At the horizon itself, it selects the checkpoint immediately
before it, keeping events at the horizon eligible. The initial snapshot is the
exception: it precedes all supplied events, including its own timestamp.
It rejects times before retained history. `Playback::begin()`
lends the working checkpoint and an event iterator together. Callers limit that
iterator to their target before application. `commit(callback)` supplies
restricted `Commit` access, not the underlying store, and prepares the next
interval. The working snapshot remains the same across intervals.

- `query_at` uses no-op commits and never changes retained state.
- `replay` retains rebuilt checkpoints through an inclusive target.
- `forward` starts from Store's latest valid checkpoint through the horizon's
  predecessor and applies remaining events through that predecessor. It runs
  the forwarding hook after each consumed timestamp and at the final predecessor, retains
  that checkpoint, and publishes the horizon without changing dirty. It deletes
  neither events nor checkpoints. Targets whose predecessor is before the
  existing horizon are rejected by Store; a missing predecessor is an error.

The forwarding hook must preserve observable state and remain within its supplied time.
`Timestamp::previous` supplies a checked immediate predecessor; consumers
implement it for their own timestamp type. Forwarding to T produces an ordinary checkpoint at T-1,
including across event-free gaps. Events at T remain available for replay.
Store rejects playback requests before the horizon. Playback resumes after the
selected checkpoint without separately filtering the horizon; forwarding has
already incorporated the older events. Future cleanup must preserve the
checkpoint at T-1. No special anchor type is needed.

Commit exposes `checkpoint_index`, `replace_checkpoint`, `forward_checkpoint`,
`push_checkpoint`, `set_dirty`, and `set_horizon`. Forwarding commits progress
from the current index through adjacent overtaken slots and replace the last
one; they do not search the checkpoint collection again. Writes use the working
checkpoint and update Playback's current index. Callers own ordering and
validity: retain completed state before publishing its boundary. Replay
replaces existing slots and appends at the end; overtaken stale slots are
replaced with equivalent state until a later cleanup can deduplicate them.
Application and forwarding callbacks are not transactional; external effects
cannot be rolled back if a callback panics.

## Unit benchmark ballpark

Measured locally on ARM64 with Rust 1.95.0, release mode, on 2026-09-24.
These are rounded Criterion point estimates from one complete run (30 samples,
200 ms warm-up, 1 second measurement per case), not performance guarantees.

Throughput below is workload count divided by elapsed seconds, derived from the
displayed timings rather than a new measurement. M = million, B = billion.
Events/s counts events applied unless labeled read or removed. Operations that
do not process events use calls/s; those rates are not event-processing capacity.
These are single-thread benchmark rates, not sustained application/server limits.

| Unit | Operation | Approximate total time | Approximate throughput | Approximate time/event |
| --- | --- | ---: | ---: | ---: |
| Apply | Apply timestamp batches | 0.69 µs | 1.45 B events/s | 0.69 ns |
| Store | Construct and drop | 20 ns | 50 M calls/s | — |
| Store | Select playback | 23 ns | 43 M calls/s | — |
| Playback | Construct | 1 ns | 1 B calls/s | — |
| Playback | Begin and consume iterator | 49 ns | 2.04 B events read/s | 0.49 ns |
| Playback | Commit with no-op policy | 1.8 ns | 556 M calls/s | — |
| Commit | Replace checkpoint and set dirty | 1.9 ns | 526 M calls/s | — |
| Query | Reconstruct state | 8.1 µs | 123 M events/s | 8.1 ns |
| Replay | Apply and retain checkpoints | 8.4 µs | 119 M events/s | 8.4 ns |
| Forward | Apply, invoke hooks, and publish horizon | 8.7 µs | 115 M events/s | 8.7 ns |
| Store | Prune | 0.41 µs | 1.22 B events removed/s | 0.82 ns |
| Store | Prune larger history | 2.4 µs | 2.08 B events removed/s | 0.48 ns |

Total times cover 1,000 events for Apply, Query, Replay, and Forward; 100 events
read for Playback begin; and 500 / 5,000 events removed for the two prune cases.
Store playback selection searches 1,024 checkpoints. Prune starts with 1,000 /
10,000 events and removes 5 / 50 checkpoints as well. Time/event is the total
time divided by that event count, not the latency of an individual event.
For read/removal rows it means time per event read/removed. A dash means no events
are processed; the total time is already the time per call.

Application workloads use the shared test event's payload-summing implementation
and tiny timestamp-plus-sum snapshots. Query, Replay, and Forward use a checkpoint
interval of 100 events and include the real underlying playback/application work;
their numbers are not isolated wrapper overhead. Prepared fixture construction
and remaining-store destruction are outside their timed sections. Query's returned
allocation and its destruction are included. Prune includes actual event deletion
and compaction in the Vec-backed test event store, so other storage implementations
and expensive destructors can change its cost considerably.

For scale: Query over 10,000 events with interval 100 measured about 83 µs
(120 M events/s); rebuilding 1,000 events into existing dirty checkpoints
measured about 8.0 µs (125 M events/s).
Large real snapshots and consumer hooks are not represented by these fixtures.
Sub-nanosecond-to-few-nanosecond results are particularly sensitive to compiler
optimization and benchmark overhead. Older benchmark runs used evolving fixtures
and setup; use this table as the current baseline rather than attributing changes
from those runs to a particular code change.

`api.rs` is a delegating interface and `types.rs` defines contracts and fixtures;
neither has a separate unit benchmark. Public API integration benchmarks are a
separate layer. Reproduce all unit measurements from this crate directory:

```sh
cargo +1.95.0 test --release --lib benchmark_ -- --ignored --nocapture --test-threads=1
```

## Public API integration benchmarks

`benches/api.rs` uses only the public API with one shared timestamp-plus-sum
snapshot and a Vec-backed event store. Events have distinct timestamps and
payloads equal to their timestamp; application sums actual payloads. The checkpoint
interval is 100. Each isolated operation starts with fresh prepared state.
Fixture preparation and store destruction are outside timing; query-result
allocation/destruction is included. Construction measures `new` with an already
built event store, not event generation. Forward uses a minimal black-box hook,
not a real consumer's compaction logic. These measure full API work, not just
dispatch overhead.

Local ARM64, Rust 1.95.0 release, 2026-09-24; 30 samples, 200 ms warm-up,
1 second measurement, rounded point estimates:

| Operation | Approximate total time | Approximate throughput | Approximate time/event |
| --- | ---: | ---: | ---: |
| New | 11.6 ns | 86.2 M calls/s | — |
| Insert, ordered into a full Vec | 0.552 µs | 1.81 M events inserted/s | 552 ns |
| Query | 7.83 µs | 128 M events/s | 7.83 ns |
| Process until | 8.07 µs | 124 M events/s | 8.07 ns |
| Forward | 8.10 µs | 123 M events/s | 8.10 ns |
| Prune | 0.263 µs | 1.90 B events removed/s | 0.526 ns |
| Mixed, 10 cycles | 18.7 µs | 53.5 M timeline events/s | 18.7 ns |
| Mixed, 100 cycles | 275.5 µs | 36.3 M timeline events/s | 27.55 ns |

Query, Process until, and Forward each process 1,000 events; Forward publishes horizon
1,001. Prune removes 500 of 1,000 events plus obsolete checkpoints. Mixed runs
progress through 1,000 / 10,000 timeline events, with average cycle times of
1.87 / 2.76 µs. Mixed time/event includes all four operations per timeline event,
not just its application. New takes ownership of 1,000 prepared events without
processing them, so time/event does not apply.

Insertion was measured separately with the same release settings after adding the
API. Each iteration admits one new event after 1,000 already-processed events.
Preparation is untimed; admission includes the fixture's binary-search identity
check and Vec growth from full capacity. This is not allocation-free insertion,
nor a late-insertion benchmark. Other event-store implementations may have very
different costs. A correctness check processes and queries the inserted event
outside the timed section.

Rates use the same count/time conversion as the unit table. Construction only
takes ownership of prepared events; it does not process 1,000 events per call.
Mixed throughput counts each distinct event reached along the timeline once:
1,000 / 18.7 µs or 10,000 / 275.5 µs. Its denominator includes all four operations,
including repeated application of some events, so this is end-to-end timeline
progress, not a count of individual apply invocations or events physically pruned.

Each mixed cycle queries the next 100-event target, replays to it, forwards to
retain the most recent 50 timestamps, and prunes. The next cycle therefore queries
and replays after the preceding cleanup. The store is reset only between complete
benchmark iterations, never between cycles. All events remain preloaded in this
mixed benchmark to preserve its four-operation baseline; insertion is measured
separately. Vec prefix deletion shifts the remaining future
events: the larger mixed case has a larger backlog as well as more cycles, so its
higher average cannot be attributed solely to store age or checkpoint behavior.
These are aggregate timings, not per-cycle latency distributions.

Before measurement, the benchmark verifies each operation and every mixed cycle,
including totals after pruning and queries exactly at the retained horizon. Inputs
and results use black boxes; correctness assertions are outside measured sections.

```sh
cargo +1.95.0 bench --bench api
```

## Verification

Unit tests share test-only fixtures in `types.rs`. Integration tests use only
`SnapshotStore` and public consumer traits, exercising real playback and application.
Each integration file owns its event, snapshot, storage, and context definitions:

- `tests/query.rs`: inclusive reconstruction, gaps, initial state, and read-only isolation.
- `tests/process_until.rs`: incremental progress, complete timestamp batches, and historical reads.
- `tests/insertion.rs`: ordered, late, same-time, minimum-time, and duplicate admission,
  partial processing after multiple insertions, and horizon enforcement before/after pruning.
- `tests/forward_prune.rs`: horizon boundaries, deferred/repeated cleanup, and continued
  playback after physical deletion.
- `tests/forwarding_hooks.rs`: snapshot compaction survives queries, replay, pruning,
  and subsequent forwarding, including forwarding without event application.
- `tests/merged_event_history.rs`: ordered history assembled from distinct storage sources.

Insertion/invalidation coverage uses the public API, never private store mutation.

`apply.rs` retains its independent unit tests and benchmark.

Application benchmarks must sum event payload values into the checkpoint, not
merely count events. Comparisons between APIs must use matching event payloads,
timestamp distributions, checkpoint intervals, and setup/timing methods.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo test --release --lib benchmark_ -- --ignored --nocapture --test-threads=1
```

The checkpoints crate is verified independently. Migration of downstream
ConTime core adapters to these interfaces is a separate step.
