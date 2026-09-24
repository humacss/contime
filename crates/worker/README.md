# contime-worker

`contime-worker` owns the blocking worker receive loop, event-store insertion
scheduling, and checkpoint orchestration.

Routed input ownership is generic. The worker moves the caller-selected input
type into its event-store implementation and does not know whether the value is
owned, shared, tracked, or represented another way.

Transport ownership is generic as well. `ApplyInput` consumes one complete
worker message and `RouteInput` consumes each routed item. `ApplyBatch` and
`RoutedInput` remain optional default implementations; an orchestrator can use
one adapter type for the router output and worker input contracts.

The crate is isolated. It does not depend on `contime`, `contime-api`,
`contime-router`, or a replay implementation. An orchestrator is responsible
for adapting independently defined message types and choosing where
`contime_worker::work` runs.

## Incremental message workers

`work_messages` drains available messages, inserting complete incoming batches,
then processes one snapshot to the next distinct pending timestamp or the
advancement target, whichever is earlier. An ordered set groups pending work
by timestamp; each snapshot has one entry. Equal-time entries needing that
timestamp run before entries already complete through it, then by snapshot ID.
After each step, the worker flushes replay notifications and returns to its
message queue. There is no maximum processing range.

Each message-worker snapshot owns one implementation of the worker's
`SnapshotStore<I>` trait. Core supplies the adapter; the worker has no dependency
on the concrete snapshot or event store crates. The adapter owns insertion and
invalidation and processes through the worker-selected timestamp with
`process_until`. The adapter exposes the incoming event's timestamp through
`event_time`; it does not supply a storage-derived next replay timestamp.
The worker tracks the earliest pending boundary and latest accepted timestamp
per snapshot. Unfinished work moves into the destination bucket; work complete
through the advancement target waits for a later advance. Work is removed once
processing reaches the latest accepted event. These boundaries are conservative:
an empty interval can remain scheduled without inspecting the stored events.
Late inputs invalidate the complete same-time bucket and its suffix.
Duplicate insertions return `changed = false` without invalidating progress.
Request completion means insertion finished, not that replay is complete.

The target starts at zero. `Advance` raises it monotonically; replay callbacks
never process buckets beyond that target. Retained future inputs do not
prevent idle. Snapshot and event queries use the same message queue and run
before further computation. Snapshot reconstruction is read-only and neither
advances checkpoints nor changes scheduled work.

Optional `Coordination` reports the earliest pending time after a `Fence` from
every distinct router for a round. Reports happen between operations and
callbacks, without waiting for idle. Duplicate and older fences are ignored.
Only explicit `Prune` messages move the retained horizon: the caller must prove
it safe, and the worker asserts that no pending timestamp precedes it. Events
strictly before the horizon are folded into checkpoint anchors and pruned.
Pruning is queued work: one snapshot history is pruned at a time, with incoming
messages handled between histories. Completion and idle wait for queued pruning
to finish; a single history's reconstruction and retention hooks are not interrupted.
The optional coordination callback reports the completed horizon only after the
whole pass succeeds, never from a drop handler or a failed retention hook.
The worker passes its admission horizon to every insertion, so existing stores
also reject older events while their forwarding/pruning jobs are still queued.
Each pruning step calls the store's `forward` followed by `prune`.
New histories inherit that admission horizon. `Advance` never infers pruning;
the retained `history_retention` argument is ignored.

The non-message `work` API remains a whole-history apply loop: it coalesces
ready batches and updates each changed snapshot once per cycle. Input and
checkpoint ownership and admission policy remain supplied
by the orchestrator.

## Worker configuration

`work_messages` accepts an ordinary work receiver and a registration receiver
carrying `Sender<bool>` subscribers (`false` = idle, `true` = working).
Pass `crossbeam_channel::never()` when no activity subscriptions are needed.
The idle loop owns registrations and notifications. Working is published before
dequeue; idle only after replay/hooks, listener flush, and locally buffered work
finish. The working loop does no activity management. The worker crate owns no
global idle state and does not depend on Core.


`WorkerConfig` retains legacy scheduling fields for source compatibility;
message workers always yield after one snapshot-processing call. Pending runnable
computation counts as working even if no messages remain. Worker exit closes
activity subscriptions so the orchestrator can detect a stopped component.

## Verification and performance (2026-09-24)

Measured on macOS arm64 with Rust 1.95.0, optimized builds and Criterion.
These are worker-overhead measurements with storage stubs, not real event-store,
snapshot-store, physics, or end-to-end core performance.

### Message-worker integration benchmarks

The public `work_messages` loop runs synchronously on the benchmark thread.
Fixture/channel construction and input generation are excluded. Timing includes
worker/store creation, channel receives, priority-queue work, stub calls,
responses, follow-up message sends, and teardown; no OS worker-thread startup or
router is included. The store stub uses a FIFO and observable running sums,
not real history insertion, deduplication, checkpoint replay, or retention.

Each scenario first checks accepted/processed event counts, sum, processing-call
count, query responses, forwarding and pruning counts outside the timed loop.
Inputs and results pass through `black_box`. Figures are approximate Criterion
point estimates (20 samples, 200 ms warmup, 1 s measurement).

| Operation | Total time | Input throughput | Time/input event |
| --- | ---: | ---: | ---: |
| Insert 1,000 future events; no processing | 20.80 µs | 48.09M/s | 20.80 ns |
| Insert/process 1,000 events, one snapshot/time | 22.05 µs | 45.36M/s | 22.05 ns |
| Insert/process 1,000 events, distinct times | 22.20 µs | 45.04M/s | 22.20 ns |
| Insert/process 10,000 events, distinct times | 221.55 µs | 45.14M/s | 22.16 ns |
| Insert/process 1,000 events, 1,000 snapshots | 177.61 µs | 5.63M/s | 177.61 ns |
| Insert/process 1,000 events + 1,000 query pairs | 118.36 µs | 8.45M/s | 118.36 ns |
| Insert/process/prune 1,000 snapshots | 190.06 µs | 5.26M/s | 190.06 ns |
| Mixed: 100 rounds, 10,000 events on 10 stores | 469.46 µs | 21.30M/s | 46.95 ns |

A query pair is one snapshot query and one event query, each returning one
stub result. The query and pruning rows include insertion and processing;
they are **not isolated query/prune latencies**. Throughput always counts input
events, not query requests or internal replay operations.

The mixed case reuses the same stores for 100 rounds, each with 100 events,
10 snapshot queries, 10 event queries, and forwarding/pruning all 10 stores.
The next round is enqueued by the completed processing callback, so this
exercises repeated message handling rather than merely coalescing all rounds.
All stages are prebuilt outside timing; enqueuing them is timed.

Distinct-timestamp processing scales approximately linearly from 1,000 to
10,000 events (10.0× time for 10× events). Many snapshots cost more because the
worker creates more stores and schedules separate snapshot/time steps.
No obvious superlinear behavior appeared in this matrix. This does not prove
long-running memory bounds or real-store performance.

These results use worker-owned bucket scheduling. Against the preceding
event-at-a-time scheduler with the same input fixtures, 1,000 distinct-time
events improved from 53.98 to 22.20 µs because they now share one processing
call. Shared-time work rose from 18.33 to 22.05 µs, and mixed work from 398.96
to 469.46 µs. Worker-owned scheduling adds bookkeeping; it is not uniformly
faster. The benchmark checks processing-call counts rather than event-timestamp
batch counts, since a call can now cover multiple timestamps.

### Unit benchmarks

These are preceding baseline results from the inline unit suite, not remeasured
after the scheduling change. Rate denominators
are explicit: history/checkpoint operations are not advertised as event
application throughput when their underlying implementation is stubbed.

| Unit | Operation | Total time | Throughput | Time/item |
| --- | --- | ---: | ---: | ---: |
| Listen | Check with no listeners | 1.62 ns | 616M checks/s | 1.62 ns/check |
| Listen | Check nonmatching collection | 4.05 ns | 247M checks/s | 4.05 ns/check |
| Listen | Register 1,000 IDs | 43.25 µs | 23.12M IDs/s | 43.25 ns/ID |
| Listen | Notify 1,000 matching IDs | 6.69 µs | 149.4M IDs/s | 6.69 ns/ID |
| Query | Return one snapshot | 43.05 ns | 23.23M queries/s | 43.05 ns/query |
| Query | Return 1,000 event handles | 1.27 µs | 790.1M handles/s | 1.27 ns/handle |
| Events (legacy) | Insert 1,000 inputs | 4.06 µs | 246.2M inputs/s | 4.06 ns/input |
| Checkpoints (legacy) | One update over a 1,000-event stub | 103.3 ns | 9.68M updates/s | 103.3 ns/update |
| Advance (legacy) | Prune 1,000 clean histories | 6.92 µs | 144.4M histories/s | 6.92 ns/history |
| Advance (legacy) | Forward/prune 1,000 histories | 6.96 µs | 143.6M histories/s | 6.96 ns/history |
| Advance (legacy) | Replay/forward/prune 1,000 histories | 14.04 µs | 71.22M histories/s | 14.04 ns/history |
| Work (legacy) | 100 batches × 1,000 inputs | 451.26 µs | 221.6M inputs/s | 4.51 ns/input |

The empty-rejection-extension benchmark was removed: its work optimized away
and the sub-nanosecond result did not measure useful worker behavior.

### Legacy public worker benchmarks

`worker_settings` continues to cover the separate batch-only `work` entry
point. The ignored replay-budget sweep was removed; it measured identical
behavior under four names. It now measures one coalesced case per shape, plus
owned/shared input variants.

| Operation | Total time | Throughput | Time/input event |
| --- | ---: | ---: | ---: |
| 1,000 one-input batches, one snapshot | 97.06 µs | 10.30M/s | 97.06 ns |
| 1,000 four-input batches, four snapshots | 86.18 µs | 46.42M/s | 21.54 ns |
| 1,000 owned 64-byte inputs | 64.28 µs | 15.56M/s | 64.28 ns |
| 1,000 shared 64-byte inputs | 74.94 µs | 13.34M/s | 74.94 ns |
| 1,000 owned 1,008-byte inputs | 83.78 µs | 11.94M/s | 83.78 ns |
| 1,000 shared 1,008-byte inputs | 86.82 µs | 11.52M/s | 86.82 ns |

The first legacy case was noisy (84–108 µs estimate interval). These fixtures
use different message shapes and storage stubs than the message-worker matrix;
do not interpret the tables as a before/after speedup.

### Reproduce

Run from the repository root. Set `CRITERION_HOME` to a writable output directory
if the default crate-local `target/criterion` is unavailable.

```sh
cargo +1.95.0 test --offline --manifest-path crates/worker/Cargo.toml --all-targets
cargo +1.95.0 test --release --offline --manifest-path crates/worker/Cargo.toml --lib benchmark -- --ignored --nocapture --test-threads=1
cargo +1.95.0 bench --offline --manifest-path crates/worker/Cargo.toml --bench messages
cargo +1.95.0 bench --offline --manifest-path crates/worker/Cargo.toml --bench worker_settings -- --sample-size 20 --warm-up-time 0.2 --measurement-time 1
cargo +1.95.0 clippy --offline --manifest-path crates/worker/Cargo.toml --all-targets -- -D warnings
```

Run timing suites serially to avoid contention. Unit tests and integration tests
are 28 and 15 respectively; the eight ignored inline benchmark entry points run
separately. Both public benchmark targets also run smoke checks under
`test --all-targets`.

## Source units

- `listen.rs`: listener registration, replay notification, disconnected sender cleanup.
- `query.rs`: snapshot/event query dispatch through the store contract.
- `work.rs`: message-priority timestamp scheduling and the blocking receive loop;
  also the retained legacy batch-only loop.
- `types.rs`: worker storage and transport contracts.
- `events.rs`, `checkpoints.rs`: legacy batch-only insertion/update helpers.
- `advance.rs`: test-only legacy horizon orchestration.
- `tests/incremental.rs`: storage-stub coverage for ordering, rewind, queries,
  target bounds, activity, fences, duplicates, pruning and repeated mixed work.
- `tests/worker_settings.rs`: legacy coalescing behavior.
- `benches/messages.rs`: public message-worker overhead with storage stubs.
- `benches/worker_settings.rs`: legacy batch-worker and input-ownership benchmarks.

The uncompiled legacy `queue.rs` and `schedule.rs` files and their unused
`priority-queue` dependency were removed; they remain available in Git history.
Concrete snapshot-store integration belongs in core.
