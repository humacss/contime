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

`work_messages` drains available messages, then computes one complete timestamp
bucket for one snapshot. A keyed priority queue orders pending snapshots by
their actual next timestamp and snapshot ID. Each snapshot has at most one
queue entry. After each step, the worker flushes replay notifications and
returns to its message queue.

Apply messages insert canonical history and immediately invalidate affected
checkpoints through `IncrementalCheckpoints::invalidate`. The checkpoint
adapter acknowledges that history changes have transferred to scheduling;
`next_time` tracks unfinished replay independently. Late inputs invalidate the
complete same-time bucket and its suffix. Request completion means insertion
and invalidation finished, and does not wait for replay.

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
New histories inherit that admission horizon. `Advance` never infers pruning;
the retained `history_retention` argument is ignored.

The non-message `work` API remains a whole-history apply loop: it coalesces
ready batches and updates each changed snapshot once per cycle. Input and
checkpoint ownership, memory accounting, and admission policy remain supplied
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
message workers always yield after one timestamp bucket. Pending runnable
computation counts as working even if no messages remain. Worker exit closes
activity subscriptions so the orchestrator can detect a stopped component.

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

Local release-mode Criterion results on 2026-09-03 compare immediate replay
with ready-batch coalescing. All 1,000 batches are queued before worker entry;
the checkpoint implementation records one inexpensive update per replay.

| Public workload | Immediate replay | Coalesced cycle | Improvement |
| --- | ---: | ---: | ---: |
| 1,000 batches, one snapshot/input | 113.70 us | 88.778 us | 21.9% |
| 1,000 batches, four snapshots/inputs | 170.07 us | 115.64 us | 32.0% |

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
- `work.rs`: message-priority timestamp scheduling and the blocking receive loop.
- `tests/incremental.rs`: real-history/checkpoint coverage for ordering, rewind,
  query priority, target bounds, activity, fences, and explicit pruning.
- `tests/worker_settings.rs`: public replay-budget and deadline behavior.
- `benches/worker_settings.rs`: end-to-end worker configuration benchmarks.

Each executable unit contains inline unit tests and an ignored inline
Criterion benchmark.
