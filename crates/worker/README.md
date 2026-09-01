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

## Initial scope

- Receive complete apply batches without owning the worker thread.
- Insert each routed input directly into its snapshot event store.
- Keep canonical event insertion separate from checkpoint materialization.
- Prefer the dirty snapshot with the largest pending input count.
- Override that preference when the oldest dirty snapshot reaches the
  configured maximum dirty age.
- Track both the actual pending count and the count last written to the count
  heap. A stale heap count may conservatively overestimate the actual count;
  the heap is updated only when the actual count exceeds it.
- Treat the count heap and deadline deque as disposable indexes over canonical
  pending-count and dirty-time state. Popped stale count entries are discarded
  or repaired from canonical state before selection.
- Compact stale deadline entries in one order-preserving pass when their count
  exceeds both the configured lower bound and active-snapshot multiplier.
- Replay a configurable number of non-overdue snapshots after each received
  batch. Deadlines and disconnected-input draining remain mandatory.
- Complete a request only after every snapshot changed by it has replayed.
- Serve snapshot and event-history queries from the same worker queue.
- Reconstruct query-local snapshots without forcing or reprioritizing replay.
- Clone event handles before returning them across a thread boundary.
- Register timestamped consumer-owned listener collections independently from
  snapshot history and emit one batched notification per collection per worker
  replay pass.

Input and checkpoint ownership, memory accounting, and admission policy remain
orchestrator concerns. The worker only coordinates the implementations supplied
through its event-store and checkpoint traits.

The receive timeout is always derived from the oldest dirty timestamp. A batch
arrival inserts its history and triggers up to `replays_per_receive` replays. A
timeout replays every snapshot whose deadline has passed before calculating the
next deadline.

Worker-local time advancement is monotonic. The worker derives its horizon
from `current_time.saturating_sub(history_retention)`, forces replay only for a
scheduled history whose dirty time is strictly before that horizon, advances
an existing checkpoint store, and then prunes events. Histories first seen
afterward are initialized with the active horizon. Equal and older requests
are successful no-ops, and sender closure signals completion.

## Worker configuration

`WorkerConfig` supplies the maximum dirty age, replay budget per received
batch, deadline-compaction lower bound, and
deadline-compaction multiplier. A replay budget of zero accumulates work until
a deadline or disconnection. Larger budgets make more checkpoint progress per
received batch.

Deque compaction reduces logical length and reuses the existing allocation; it
does not return high-water capacity to the allocator. Capacity shrinking is a
separate policy that remains deferred.

## Benchmark snapshot

Snapshot-listener unit results recorded on 2026-09-01:

| Worker-local operation | Total | Amortized |
| --- | ---: | ---: |
| Register one collection with 1,000 IDs | 58.069 us | 58.069 ns/ID |
| Replay check, no collections | 2.1286 ns | 2.1286 ns/replay |
| Replay check, one nonmatching collection | 6.2251 ns | 6.2251 ns/replay |
| Accumulate + flush 1 matching snapshot | 54.117 ns | 54.117 ns/ID |
| Accumulate + flush 100 matching snapshots | 994.68 ns | 9.947 ns/ID |
| Accumulate + flush 1,000 matching snapshots | 8.7402 us | 8.740 ns/ID |

Registration deduplicates one collection's IDs, sends one batched `Registered`
message, and attaches a compact generational collection ID to each worker-local
snapshot slot. Replay notification inspects only memberships on snapshots that
actually replayed, filters them by watched timestamp, and sends one batched
`Replayed` message per touched collection. The empty case shows the fast path
when no collection has been installed.

Horizon orchestration results for 1,000 worker-local histories recorded on
2026-09-01:

| Workload | Total | Histories/s |
| --- | ---: | ---: |
| Clean event pruning | 122.4 us | 8.17 million |
| Checkpoint anchor + pruning | 127.9 us | 7.82 million |
| Forced replay + anchor + pruning | 544.6 us | 1.84 million |

Query unit results recorded on 2026-09-01:

| Worker-local query | Total | Amortized |
| --- | ---: | ---: |
| One found snapshot | 59.66 ns | 59.66 ns/result |
| 1,000 found event handles | 1.571 us | 1.57 ns/result |

The snapshot case includes one history lookup, query-local reconstruction by
the supplied checkpoint implementation, boxing, and the response callback. The
event case includes one history lookup, range filtering, cloning 1,000 handles,
and the response callback. Neither case includes router or API transport.

Local release-mode Criterion results on 2026-08-29:

| Public workload | Replays per receive | Time | Routed inputs/s |
| --- | ---: | ---: | ---: |
| 1,000 batches, one snapshot/input | 0 | 354.77 us | 2.82 million |
| 1,000 batches, one snapshot/input | 1 | 164.18 us | 6.09 million |
| 1,000 batches, one snapshot/input | 4 | 187.52 us | 5.33 million |
| 1,000 batches, one snapshot/input | 16 | 187.54 us | 5.33 million |
| 1,000 batches, four snapshots/inputs | 0 | 1.0931 ms | 3.66 million |
| 1,000 batches, four snapshots/inputs | 1 | 385.83 us | 10.37 million |
| 1,000 batches, four snapshots/inputs | 4 | 443.26 us | 9.02 million |
| 1,000 batches, four snapshots/inputs | 16 | 461.30 us | 8.67 million |

One replay per receive was fastest in both current integration workloads. The
setting remains configurable because replay cost and snapshot fan-out are
consumer-dependent.

### Input ownership comparison

The generic ownership benchmark processes 1,000 one-input batches across four
snapshots with one replay per receive. Input construction occurs outside the
timed routine. `shared` is a benchmark-local one-pointer wrapper around
`Arc<Event>`; production worker code does not refer to `Arc`.

| Event bytes | Owned total | Owned throughput | Shared total | Shared throughput |
| ---: | ---: | ---: | ---: | ---: |
| 64 | 199.76 µs | 5.006M/s | 165.95 µs | 6.026M/s |
| 208 | 158.13 µs | 6.324M/s | 177.98 µs | 5.619M/s |
| 1,008 | 192.15 µs | 5.204M/s | 190.59 µs | 5.247M/s |

The worker does not fan inputs out or clone them, so these results show no
stable relationship between payload size and ownership strategy. Scheduling,
event insertion, checkpoint updates, and completion
dominate this workload. Pointer ownership is selected for efficient router
fan-out and retained event history, not because it intrinsically accelerates
the worker loop.

### Pipeline comparison

The independently measured Arc/shared fast paths currently have the following
approximate throughput:

| Boundary | Throughput |
| --- | ---: |
| API, 1,000 already-shared inputs | 1.9–2.1 billion inputs/s |
| Router, 64-byte-or-larger shared events | 122–148 million routes/s |
| Worker, one replay per receive | 6.09–10.37 million routed inputs/s per worker |

The worker is therefore the narrowest single instance, as expected for the
stage that owns event stores and performs checkpoint updates. Dividing router
route throughput by the measured worker range gives capacity for roughly
13–22 equally loaded workers before routing becomes the next bottleneck. At 20
workers, the lower worker measurement corresponds to about 122 million routed
inputs/s, still within the measured router range.

These are boundary-specific microbenchmarks rather than one end-to-end
measurement. The API shared-input benchmark excludes downstream receipt, the
router benchmark excludes worker execution, and the worker fixtures use cheap
in-memory event and checkpoint implementations. The comparison is useful for
capacity direction, not a promise of aggregate application throughput.

An isolated order-preserving compaction from 2,001 deadline entries to 1,000
cost about 9.93 us. Across 1,000 single-snapshot reactivation cycles, lower
bounds of 64, 256, and 1,024 measured approximately 50.3 us, 49.5 us, and 47.1
us respectively; 1,024 is the best current starting point.

## Source units

- `queue.rs`: keyed priority-queue operations and their isolated unit
  benchmarks.
- `schedule.rs`: dirty-time and pending-count scheduling policy.
- `events.rs`: event-store creation, insertion, and dirty scheduling.
- `checkpoints.rs`: checkpoint materialization and request completion.
- `listen.rs`: listener registration, replay notification, and disconnected
  sender cleanup.
- `work.rs`: the deadline-driven blocking receive loop.
- `tests/worker_settings.rs`: public replay-budget and deadline behavior.
- `benches/worker_settings.rs`: end-to-end worker configuration benchmarks.

Each executable unit contains inline unit tests and an ignored inline
Criterion benchmark.
