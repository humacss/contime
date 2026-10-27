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

## API guide

[`SnapshotStore`](https://github.com/humacss/contime/blob/main/crates/snapshots/src/api.rs) is the public entry point. Implement the contracts
in [`types.rs`](https://github.com/humacss/contime/blob/main/crates/snapshots/src/types.rs): `Snapshot` for clonable state, `Event` for timestamp
access, `EventStore` for canonical borrowed history, and `Apply` for complete
nonempty timestamp batches. Implement `InsertEventStore` to admit later events
and `Timestamp` to support forwarding. None requires a specific event-storage
crate or execution topology.

- `query` returns an owned reconstruction through an inclusive target without
  retaining checkpoints or moving processing progress. Use an effect-free
  context because application callbacks still run. In gaps, the returned snapshot
  keeps the last applied or starting-checkpoint timestamp.
- `process_until` retains reconstructed progress through an inclusive target.
  Whole timestamp batches are never split by checkpoint intervals.
- `insert` admits without applying and invalidates affected checkpoints only for
  `Insert::Inserted`. Events at the retention horizon remain admissible.
- `forward` retains state immediately before a new horizon and invokes consumer
  compaction hooks. It leaves events at the horizon available for replay.
- `prune` physically removes obsolete history after forwarding, preserving the
  predecessor checkpoint. Repeating cleanup is harmless.

The optional `counters` feature adds `process_until_counted`, which performs the
same replay and returns the number of raw canonical events traversed during that
call. The count includes drained events ignored by application logic and prefixes
reconstructed after late insertion. Query and forwarding work are excluded. It
uses the working checkpoint's existing event-count delta without another history
scan. `process_until` keeps its existing API and behavior.

The initial snapshot precedes the supplied events, including those at its own
timestamp, which becomes the initial horizon. A target before retained history
returns `NoCheckpoint`. Forwarding also fails if its horizon has no predecessor,
or that predecessor is before retained history; repeating the current horizon
is therefore an error. Application and hook effects are not transactional.

`BatchLookahead::next_event_time` may be read only after the current iterator is
exhausted or dropped. It reports the next timestamp exposed by that playback
range, not a guarantee about all retained or future history. Processing exposes
additional lookahead across a completed checkpoint interval; query and forwarding
do not supply that extra interval lookahead.

Generate the public Rust API reference from the workspace root:

```sh
cargo doc -p contime-snapshots --no-deps --open
```

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

## Benchmarks

Inline benchmarks cover snapshot application and playback operations. The `api`
benchmark exercises the public storage API. Compare matching event payloads,
timestamp distributions, checkpoint intervals, and setup/timing boundaries.

```sh
cargo test --release -p contime-snapshots --lib benchmark_ -- --ignored --nocapture --test-threads=1
cargo bench -p contime-snapshots --bench api
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

The snapshot store's contracts can be verified independently of downstream
adapters and worker scheduling.
