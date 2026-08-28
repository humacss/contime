# Minimal Prepared Apply Pipeline Design

Date: 2026-08-28
Status: Approved in conversation; pending written-spec review

## Purpose

ConTime's input path will have three small responsibilities:

1. the API groups owned inputs into their final per-snapshot batches;
2. the router hashes each snapshot ID and moves complete batches into final
   per-worker messages; and
3. each worker directly applies those batches to the corresponding snapshot
   histories.

No later layer will reopen routes, regroup events, clone payloads, or build an
equivalent temporary representation. Synchronous completion will be signalled
by dropping the final request-scoped sender rather than by sending an empty
success value from every worker.

## Goals

- Keep the production worker as a plain blocking receive loop.
- Perform all event-to-snapshot grouping once at the API boundary.
- Move owned batches through the router and worker without payload cloning.
- Clone one input only when it genuinely targets more than one snapshot, and
  then exactly once per additional target.
- Remove temporary snapshot-ID vectors and equivalent intermediate batch
  representations.
- Hash each prepared snapshot ID exactly once in the router.
- Send at most one message to each affected worker for one API request.
- Apply each worker message directly to snapshot histories without worker-side
  grouping.
- Correct the benchmarks so worker construction, first dispatch, first history
  creation, and initial retained-storage allocation are outside the timed
  steady-state apply.

## Non-goals

- Custom spinning, polling, backoff, worker-temperature tracking, thread
  pinning, or other scheduling policy above the channel library.
- Cross-worker transactions or rollback under memory pressure.
- Changing canonical history ordering, replay, checkpoints, or horizon logic.
- Optimizing Timeless Runtime or Spacetime in this pass.
- Preserving request arrival order across independent snapshot histories.

## Semantic ordering

API iteration order, hash-map iteration order, router order, worker scheduling,
and the order in which independent histories are called have no semantic
meaning. Every snapshot history derives its state from its canonical
`(time, input_id)` order. The API grouping map therefore does not preserve
first-seen snapshot order.

Inputs carrying the same retained ID are required by contract to represent the
same input. Cross-worker memory exhaustion can still produce partial
application; deterministic transactional admission remains a separate future
concern rather than a reason to order this hot path.

## Ownership contract

The pipeline forwards ownership, not borrowed references. Worker threads may
outlive the API stack frame, and snapshot histories retain their inputs.

For an input with `N` destination snapshots:

- `N == 0`: discard the unrouted input;
- `N == 1`: move the input once and perform zero input clones;
- `N > 1`: clone the input exactly `N - 1` times and move the original into the
  final destination.

The router and worker perform zero input clones. Moving a batch or `Vec` moves
its allocation descriptor; it does not copy its elements. Containers allocated
for the prepared request, final worker messages, and retained history are
authoritative storage, not temporary duplicate representations.

`Arc<Input>` is not the default representation. It would impose an allocation
and atomic reference counting on every single-route input to avoid clones only
for the multi-route case.

## API preparation

The API consumes the input iterator and constructs the actual router request:

```rust
struct PreparedRequest<IL> {
    snapshots: AHashMap<u128, SnapshotInputBatch<IL>>,
    conservative_bytes: u64,
}

struct SnapshotInputBatch<IL> {
    snapshot_id: u128,
    inputs: Vec<IL>,
    conservative_bytes: u64,
}
```

The map is not a temporary index into a second batch collection. Its values are
the complete owned batches that the router will consume.

For each input, the API visits snapshot IDs directly. It does not collect them
into a temporary vector. To move the original input into the final route, it
keeps only one pending ID:

```rust
let mut pending = None;
input.visit_snapshot_ids(&mut |snapshot_id| {
    if let Some(previous) = pending.replace(snapshot_id) {
        snapshots.entry(previous).or_default().push(input.clone());
    }
});
if let Some(final_id) = pending {
    snapshots.entry(final_id).or_default().push(input);
}
```

The production implementation will express this without exposing internal
batches publicly. Conservative retained-memory estimates are accumulated while
the API is already building each final batch. The API-wide advisory memory
check therefore requires no second event-routing pass.

There is one uniform grouping path. It does not special-case a one-input or
one-snapshot request merely to avoid the grouping map.

## Router partitioning

The router receives `PreparedRequest` and never visits an individual input. It
creates one slot per configured worker, with message storage initialized only
for affected workers:

```rust
struct WorkerInputBatch<IL> {
    snapshots: Vec<SnapshotInputBatch<IL>>,
    conservative_bytes: u64,
    completion: Completion,
}
```

For every entry drained from the API map, the router:

1. computes `hash(snapshot_id) % worker_count` exactly once;
2. initializes that worker's message if necessary;
3. moves the complete snapshot batch into the message; and
4. adds the already-calculated conservative bytes to the worker total.

It then moves each affected worker message into that worker's channel. The
router does not clone inputs or batches, reopen routes, apply admission policy,
look up histories, or wait for completion.

## Completion contract

Both asynchronous and synchronous application use a request-scoped completion
sender carried by the final worker messages.

For caller-managed asynchronous dispatch, the caller supplies the sender and
retains the receiver. The API consumes that sender, the router clones it once
per affected worker message, and the API/router drop their originals after
dispatch.

Synchronous `apply` creates its own `(sender, receiver)`, dispatches through the
same path, drops its original sender, and consumes actual rejection messages
until the channel disconnects.

Workers send only non-empty rejection data. Successful workers send nothing.
Finishing a worker message drops that message's sender. The last affected
worker therefore signals request completion by dropping the final sender.
There is no affected-worker response count and no explicit empty success
vector.

Concurrent synchronous calls remain isolated because each call owns a distinct
channel.

## Worker application

The production loop remains structurally equivalent to:

```rust
while let Ok(message) = receiver.recv() {
    reserve_message_memory(message.conservative_bytes);

    for batch in message.snapshots {
        histories
            .entry(batch.snapshot_id)
            .or_insert_with(|| new_history(batch.snapshot_id))
            .apply(batch.inputs);
    }

    send_rejections_if_any(message.completion);
}
```

There is no custom idle management outside or inside this loop. Blocking,
parking, and notification are channel-library responsibilities.

The worker performs one message-level memory reservation, one persistent
history lookup per prepared snapshot batch, and one direct history application
per batch. It does not construct snapshot messages, group by input ID, group by
snapshot ID, inspect routes, or copy input payloads.

## Memory accounting

The existing provisional memory policy remains:

- API preparation accumulates a conservative request estimate and performs an
  advisory whole-request check.
- The router carries per-batch estimates into per-worker totals while it is
  already partitioning batches.
- Each worker attempts one reservation for its complete message.
- Histories report actual retained deltas and release conservative excess.
- A worker that cannot reserve its message rejects that complete worker
  message without applying a prefix.

Concurrent requests can pass the advisory API check and later receive partial
cross-worker rejection. This limitation remains documented.

## Benchmark contract

The previous Runtime fixture called `query_at` with an empty snapshot-ID list as
a warm-up. That call returns at the API boundary and never reaches a worker, so
it does not warm worker startup or dispatch.

Every steady-state boundary fixture will instead:

1. create its ConTime/router/worker/history fixture outside timing;
2. apply one real warm-up input outside timing;
3. use the same input type and snapshot ID for the measured input;
4. give the measured input a distinct ID and later timestamp; and
5. avoid crossing a checkpoint boundary between warm-up and measurement.

The benchmark matrix will include matched one-input and 1,000-input workloads
at these boundaries:

1. public API;
2. router with an already-prepared API request;
3. worker with an already-partitioned worker message; and
4. snapshot history with an already-prepared snapshot batch.

The one-input benchmark measures the real synchronous path after initialization.
The 1,000-input benchmark shows fixed overhead amortization and per-input
history cost. Criterion setup must not use an empty query as a synchronization
barrier.

Benchmark reports will state whether a number includes API grouping, router
hashing, channel dispatch/completion, worker memory accounting, history lookup,
and history application. The target is to move warmed one-input public apply
toward the direct-history cost plus unavoidable routing and channel overhead;
the implementation will report measured results rather than claim a fixed
latency in advance.

## Verification

Tests written before production changes will cover:

1. one-route input performs zero input clones;
2. an input with three routes performs exactly two input clones;
3. unrouted inputs create no snapshot batch;
4. the API map contains one complete batch per distinct snapshot ID;
5. arbitrary API map iteration produces identical queried snapshot state;
6. router partitioning hashes every prepared snapshot ID once and sends at most
   one message per affected worker;
7. router and worker perform zero input clones;
8. the worker directly applies each already-prepared snapshot batch;
9. a fully successful request closes its completion channel without sending an
   empty result;
10. rejection data arrives before the final completion disconnect;
11. multiple affected workers close the channel only after all complete;
12. concurrent synchronous calls cannot consume one another's rejections;
13. advisory and worker memory rejection behavior remains unchanged; and
14. replay, late insertion, duplicate identity, markers, checkpoints, horizon
    pruning, generic time, queries, and advancement retain their behavior.

## Documentation and compatibility

README flow documentation will use:

```text
API inputs -> snapshot batch map -> worker messages -> snapshot histories
```

It will remove claims that API batching preserves first-seen snapshot order and
will explain completion-by-disconnection. Benchmark tables will be replaced
only with fresh measurements from the corrected fixtures.

This is an intentional internal and unstable-API cleanup. Any caller-managed
completion signature changes required to remove response counts will be updated
in direct consumers in a separate coordinated pass unless those consumers are
explicitly included in the implementation scope.
