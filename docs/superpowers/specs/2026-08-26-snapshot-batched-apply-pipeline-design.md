# Snapshot-Batched Apply Pipeline Design

Date: 2026-08-26

## Summary

ConTime will transform API inputs into snapshot batches once, route complete
snapshot batches to their owning workers, and apply those batches directly to
snapshot histories. Snapshot histories, rather than workers, will own retained
input identity. The canonical input-inspection journal will be removed.

This replaces the current expand-and-regroup pipeline, in which the router emits
one routed input per snapshot, the worker groups those inputs by event ID,
duplicates them into a journal, groups them again by snapshot ID, and finally
calls snapshot history.

## Goals

- Make the data shape passed to workers match the data shape consumed by
  snapshot histories.
- Perform snapshot grouping exactly once, at the API boundary.
- Route each complete snapshot batch to exactly one owning worker.
- Remove worker-side event-ID grouping, snapshot regrouping, and the input
  journal.
- Scope duplicate input-ID detection to each snapshot history.
- Preserve deterministic replay, late insertion, marker behavior, checkpoints,
  horizon pruning, queries, and synchronous rejection reporting.
- Reduce worker overhead toward the direct snapshot-history baseline.
- Keep memory-pressure behavior deliberately simple while documenting its
  current partial-application limitations.

## Non-goals

- Transactional application across workers or snapshots.
- Retrying partially applied requests.
- Perfect memory admission in the presence of duplicates or concurrent calls.
- A persistent event store or retained-input inspection API.
- New public batching APIs.
- Optimizing the snapshot-history algorithms beyond identity ownership changes
  required by this design.

## Current problem

The current worker receives a flat list of `(snapshot_id, input)` values. An
input routed to several snapshots appears several times. Before applying it, the
worker:

1. Groups routed values by input ID for admission.
2. Maintains a worker-wide retained-ID index and horizon index.
3. Reserves memory per input.
4. Clones each input into a separately ordered inspection journal.
5. Allocates a route vector for every journal entry.
6. Groups the admitted routed values again by snapshot ID.
7. Applies the reconstructed snapshot batches.

The matched 1,000-event, one-snapshot benchmark measured approximately
`223.47 µs` at worker entry and `52.33 µs` at direct snapshot-history entry.
A bare crossbeam worker round trip measured approximately `0.75 µs`. The
remaining worker cost therefore comes primarily from admission, journaling, and
repeated batch reconstruction rather than scheduling or snapshot history.

## Architecture

### API batching

The public APIs remain iterator based:

```rust
contime.apply(inputs)
contime.send(inputs)
```

Each call consumes its input iterator and constructs an ordered list of
snapshot batches:

```rust
struct SnapshotInputBatch<IL> {
    snapshot_id: u128,
    inputs: Vec<IL>,
    conservative_bytes: u64,
}
```

The API visits each input's snapshot IDs through `InputRoute`. It appends one
owned input lane to each selected snapshot batch. It moves the input into its
final route and clones it only for earlier routes, as required because each
snapshot history retains an owned copy.

Snapshot batches preserve the order in which snapshot IDs first appear in the
API request. Inputs within one snapshot batch preserve caller order. Unrouted
inputs are discarded and do not consume memory or produce rejections.

The builder may use a request-local hash map from snapshot ID to batch index,
but the resulting representation is an ordered `Vec<SnapshotInputBatch<IL>>`.

### Router partitioning

The router receives complete snapshot batches. It hashes each distinct
`snapshot_id` exactly once and moves the complete batch into that worker's
request bucket:

```rust
struct WorkerInputBatch<IL> {
    snapshot_batches: Vec<SnapshotInputBatch<IL>>,
    conservative_bytes: u64,
    completion: Completion<Vec<EventRejection>>,
}
```

The router preserves snapshot-batch order within each worker bucket and sends
at most one input message to each affected worker. It never visits individual
input routes, clones input lanes, splits a snapshot batch, performs admission,
or waits for worker completion.

### Worker application

The worker receives batches already grouped by snapshot ID. After memory
admission, it loops over `snapshot_batches`, gets or creates the corresponding
`SnapshotHistory`, and passes the owned input vector directly to that history.

The worker does not:

- group by input ID;
- maintain input identity;
- reconstruct routes;
- maintain an inspection journal;
- sort retained canonical inputs globally; or
- regroup inputs by snapshot ID.

The worker aggregates history rejections and completes the request once.

### Snapshot-history identity

Each `SnapshotHistory` owns an input-ID index scoped to the inputs currently
retained by that history. Applying an ID already retained by that history is a
successful no-op even if the duplicate supplies a different timestamp or
payload.

The same input ID may still be applied to another history that has not retained
it. This allows a repeated request to repair a route that was previously only
partially applied.

When horizon pruning removes an input from one history, that history also
forgets the input ID. A later input with that ID may then be accepted by that
history. Identity retention therefore follows the same horizon as the input
payload and requires no separate worker-level time index.

Inputs older than a history's retained horizon are rejected by that history as
`BeforeHistoryHorizon`. Identity checks, ordered/late insertion, replay,
checkpoint selection, and pruning remain history responsibilities.

## Memory admission

ConTime retains one global memory budget and one atomic current-usage counter.
Memory management remains conservative and intentionally provisional.

### API precheck

After building snapshot batches, the API sums their conservative memory
estimates and reads the remaining global memory. This is an advisory check, not
a reservation.

If the estimate exceeds currently available memory:

- `apply` dispatches nothing and returns each unique routed input ID with
  `EventRejectionReason::MemoryFull`;
- `send` dispatches nothing and returns `ContimeError::MemoryFull`.

Because the check is only an atomic read, concurrent requests may each pass it
before workers perform authoritative reservations.

### Worker reservation

The router carries each worker message's conservative byte total. Before
mutating any history, a worker attempts one atomic reservation for its complete
message.

If reservation succeeds, the worker applies every snapshot batch and reconciles
the conservative reservation with the actual aggregate memory delta returned
by the histories. Duplicate and stale inputs may make actual usage smaller than
the reservation; the excess is released.

If reservation fails, the worker applies no snapshot batch from that message
and returns every unique input ID in the message as `MemoryFull`. It does not
scan for a smaller prefix or attempt later events individually.

This policy requires one authoritative memory reservation per affected worker,
not one reservation per event.

### Partial application limitation

The API precheck does not reserve memory. Under concurrent load, some workers
may reserve successfully while another worker rejects its message. An input
routed to several snapshots may likewise reach only the snapshots owned by
successful workers.

Synchronous `apply` merges rejection pairs by `(event_id, reason)`. If any route
for an event is rejected, that event ID appears in the returned rejection list,
even if another route succeeded.

Asynchronous `send` performs the API precheck but returns after enqueue. Worker
rejections occurring afterward are not observable through `send`; it remains
best effort after dispatch.

Transactional cross-worker admission, compensating rollback, and retry
coordination are deferred. The README must describe this limitation explicitly.

## Error contract

- `apply` retains `Result<Vec<EventRejection>, ContimeError>`.
- A successful, fully applied request returns an empty rejection vector.
- API-wide memory precheck failure returns `Ok` with all unique routed input IDs
  marked `MemoryFull`.
- Worker message rejection returns affected input IDs marked `MemoryFull`.
- A history rejects inputs before its horizon as `BeforeHistoryHorizon`.
- Duplicate IDs within one history are successful no-ops.
- `send` retains `Result<(), ContimeError>` and gains
  `ContimeError::MemoryFull` for API-precheck rejection.
- Existing infrastructure errors remain infrastructure errors.

## Input inspection removal

The following are removed:

- `Contime::inspect_inputs`;
- `InputJournalEntry`;
- the worker input log;
- inspection worker messages;
- router inspection dispatch;
- API-side inspection merging;
- journal pruning and memory accounting;
- journal-specific tests, examples, and documentation; and
- inspection calls used only as asynchronous benchmark barriers.

Snapshot histories already retain the routed inputs required for replay.
Persistence, auditing, and canonical event inspection belong to the surrounding
event system rather than ConTime.

## Testing

Tests will be written before each production change and will cover:

1. API batching preserves first-seen snapshot order and original per-snapshot
   input order.
2. Multi-snapshot inputs enter every correct snapshot batch and unrouted inputs
   enter none.
3. Router partitioning hashes complete snapshot batches to their owning workers
   and sends at most one message per affected worker.
4. Workers apply already-grouped batches without worker-side regrouping.
5. One event ID is a no-op in a history that retains it while remaining
   admissible in another history.
6. Horizon pruning forgets identity independently per history.
7. Inputs before a history horizon return `BeforeHistoryHorizon`.
8. API memory precheck rejects the complete request without router dispatch.
9. Worker memory failure rejects the complete worker message without mutating
   any of its histories.
10. A synchronous partial cross-worker apply reports IDs rejected by any worker.
11. `send` reports API-precheck failure but remains best effort after enqueue.
12. Existing replay, markers, checkpoints, generic time, queries, memory release,
    derives, fragments, and concurrent API calls retain their behavior.
13. Public API compile checks prove the inspection types and method are absent.

## Benchmarks

The existing outside-in Criterion workload remains the primary measurement. It
applies 1,000 unique events to one snapshot through four matched entry points:

1. public API;
2. router;
3. worker; and
4. snapshot history.

The benchmark adapters and fixtures will be updated to use snapshot batches and
worker messages. Input construction, worker startup, and warm-up remain outside
the timed region.

The README will replace the current measurements with fresh 30-sample results.
The expected shape is:

- worker entry approaches snapshot-history entry because it performs only one
  message reservation, history lookup, direct apply, and completion;
- router entry adds only snapshot-batch hashing, worker bucketing, enqueue, and
  completion wait; and
- API entry includes the one intentional input-to-snapshot batching pass.

Focused history benchmarks, including late-rate, reverse-ordered batch, merged
replay, and horizon pruning, remain separate.

## Documentation

The README and crate documentation will describe the new pipeline as:

```text
API inputs -> snapshot batches -> worker messages -> snapshot histories
```

They will remove input-inspection examples and state that memory management is
a work in progress. The warning will cover conservative over-rejection,
concurrent precheck races, partial cross-worker and cross-snapshot application,
synchronous rejection reporting, asynchronous best effort, and deferred
transactional consistency.

## Compatibility

This is an intentional breaking change to an unstable crate:

- `inspect_inputs` and `InputJournalEntry` disappear;
- `ContimeError` gains `MemoryFull`;
- internal router and worker message representations change; and
- duplicate identity moves from worker scope to snapshot-history scope.

The public `apply`, `send`, `query_at`, `advance_to`, constructors, lane traits,
derive macros, and snapshot application traits otherwise retain their current
shape.
