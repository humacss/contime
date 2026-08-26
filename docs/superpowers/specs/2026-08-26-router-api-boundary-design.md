# ConTime Router and API Boundary Design

Date: 2026-08-26  
Status: Approved in conversation; pending written-spec review

## Purpose

Restore the router to one responsibility: partition operations by worker and
dispatch them. The router must never wait for worker responses and must not own
admission policy, retained input identity, history-horizon checks, journal
accounting, memory decisions, or public outcomes.

The public `Contime` API remains clean and synchronous where requested. It
creates request-scoped response channels, asks the router to dispatch, and
collects responses directly from workers. Concurrent synchronous calls remain
supported because every call owns a distinct response channel.

This design corrects the Router/API boundary first. It changes the worker
message/result seam only as required to support that boundary. Optimizing worker
history, journal, and replay internals is a separate follow-up.

## Public API contracts

```rust
pub fn send<I>(&self, inputs: I) -> Result<(), ContimeError<Time>>;

pub fn apply<I>(&self, inputs: I)
    -> Result<Vec<EventRejection>, ContimeError<Time>>;
```

`send` is completely asynchronous at the API level. It returns after the router
has enqueued every affected worker batch. It creates no response channel and no
worker response is produced.

`apply` is synchronous. It returns only after every affected worker has applied
its complete portion of the batch and sent one response. An empty rejection
vector means the batch completed without normal admission rejections.

`ContimeError` is reserved for infrastructure failures such as failure to
enqueue a worker message or disconnection of a request response channel. Normal
history and memory decisions are values, not infrastructure errors.

The same ownership rule applies to the other synchronous operations:

- `query_at` collects query results in `Contime`.
- `inspect_inputs` collects and merges retained inputs in `Contime`.
- `advance_to` waits for advancement acknowledgements in `Contime`.
- `current_time` is API state rather than router state.

The public API does not expose worker IDs, worker counts, channels, response
tokens, routing buckets, or dispatch receipts.

## Rejection contract

There is no `ApplyOutcome` wrapper and no accepted-input list.

```rust
pub struct EventRejection {
    pub event_id: u128,
    pub reason: EventRejectionReason,
}

pub enum EventRejectionReason {
    BeforeHistoryHorizon,
    MemoryFull,
}
```

The event ID is `Input::id()` for any temporal input accepted by the lane
universe, including marker inputs. The public name remains `event_id` because
that is the consumer-facing outcome vocabulary.

Rejections deliberately contain no time, snapshot ID, worker ID, payload, or
routing metadata. If an event is accepted on one route and rejected on another,
the event appears as rejected without exposing where the partial application
occurred.

The API deduplicates identical `(event_id, reason)` pairs. If one event receives
different reasons, each distinct pair remains in the result. Successful events
are not echoed. Duplicate retained events remain silent idempotent no-ops.

`EventRejection` and `EventRejectionReason` have stable equality and ordering so
aggregation can extend, sort, and deduplicate one result vector without a
temporary hash set. An empty result vector performs no heap allocation.

## Request-scoped response channels

Every synchronous API call creates one response channel for that call. This is
one channel per request, not one per worker and not one per event.

For `apply`:

1. `Contime::apply` creates `(response_tx, response_rx)`.
2. It passes `Some(response_tx)` and the input batch to the router.
3. The router partitions the batch, clones the sender once per affected worker,
   and enqueues at most one input message per affected worker.
4. The router returns the number of worker messages it successfully dispatched.
5. The API drops its original sender and receives exactly that many responses.
6. Each worker sends one `Vec<EventRejection>` after completing its full worker
   batch.
7. The API merges, sorts, and deduplicates the worker vectors.

Concurrent calls cannot cross responses because each owns a different channel.
No request ID, global response channel, shared response registry, or response
multiplexer is required.

For `send`, the completion mode is `None`; no response channel exists and the
worker performs no response work.

Queries, inspection, and advancement use operation-specific request channels
and response types. Workers respond directly to the API-owned channel. A
response never travels back through the router.

## Router responsibility

The router owns only:

- configured worker senders;
- worker count;
- the snapshot-ID hash configuration;
- allocation-free route extraction;
- per-request worker batch construction;
- dispatch and affected-worker counting.

The router performs no waiting and owns no canonical history state.

For an input batch, it:

1. Creates worker batch buckets once for the request.
2. Visits each input's target snapshot IDs without allocating a vector.
3. Computes `hash(snapshot_id) % worker_count` for each route.
4. Appends the route to that worker's request bucket.
5. Sends at most one message to every non-empty worker bucket.
6. Returns the number of messages dispatched.

The router does not:

- maintain retained input IDs;
- detect duplicates;
- validate history horizons;
- admit or reject memory;
- construct outcomes;
- record or size journals;
- maintain current time;
- create response channels;
- receive responses;
- wait for workers.

## Allocation-free route extraction

The current `snapshot_ids() -> Vec<u128>` contract is replaced by a statically
dispatched visitor interface for both generated input lanes and user-defined
marker routes:

```rust
fn visit_snapshot_ids<F>(&self, visit: &mut F)
where
    F: FnMut(u128);
```

Generated routes call the visitor directly. They do not allocate or collect
snapshot IDs.

The router keeps one reusable route scratch buffer for the request when an input
has multiple targets. It is cleared and reused rather than allocated per input.
For a one-target input, the input is moved once into its worker batch. For a
multi-target input, it is cloned only for additional routes and moved into the
final route.

Worker batch buckets are allocated once per request and use the incoming
iterator size hint for initial capacity. The design forbids per-input heap
objects such as snapshot-ID vectors, routed-worker-index vectors, or journal
route vectors in the router. Amortized growth of request-level worker buckets is
permitted.

Hashing the same snapshot repeatedly may be optimized later. This pass removes
misplaced work and per-input allocation first; it does not require a snapshot
hash cache.

## Worker message seam

Input dispatch uses one processing path with an internal completion mode:

```rust
enum Completion<T> {
    None,
    Respond(Sender<T>),
}

WorkerInbound::Inputs {
    inputs: Vec<WorkerInput<InputLanes>>,
    completion: Completion<Vec<EventRejection>>,
}
```

`Completion::None` is used by `send`. `Completion::Respond` is used by `apply`.
The worker processes both identically and responds exactly once only for the
latter.

Workers own ordinary admission decisions. Inputs before the worker's retained
horizon produce `BeforeHistoryHorizon`; inputs that cannot be retained within
available memory produce `MemoryFull`. Different workers may therefore produce
partial application. The worker returns only rejection values, never routing
metadata.

Retained identity and silent duplicate handling move to worker/history state.
This boundary pass retains the existing shared atomic memory counter. Each
worker attempts reservation while admitting its own inputs and returns
`MemoryFull` for an input it cannot reserve without crossing the configured
budget. Reservation and rejection therefore occur in workers rather than the
router, and partial application is possible. A later worker-focused design will
optimize admission, journals, grouping, and replay without changing this API
seam.

## Other synchronous operations

### Query

`Contime::query_at` creates a request channel and includes original result
positions in the routed query records. The router dispatches query batches and
returns the affected count. The API receives that many worker vectors and fills
the public result in request order.

### Input inspection

`Contime::inspect_inputs` sends one request to each relevant worker through the
router. The API receives worker journal slices, performs the canonical merge and
deduplication, and returns the public ordered result. The router does not inspect
or merge results.

### Horizon advancement

`Contime::advance_to` owns current-time synchronization, creates one request
channel, dispatches advancement to all workers, and waits for one unit response
per worker. The router only broadcasts and returns its dispatch count.

## Error and partial-dispatch semantics

An event rejection is a normal result. Infrastructure failure is an error.

If enqueueing fails after another worker batch was already dispatched, the API
returns an infrastructure error and does not claim rollback. ConTime does not
provide cross-worker transactions. This is consistent with partial application
being an accepted property of worker-local memory admission.

After successful dispatch, the API drops its original response sender before
waiting. If a worker exits without responding, channel disconnection terminates
the wait with an internal error rather than hanging indefinitely.

Worker application remains infallible by contract. Broken invariants may panic;
they are not encoded as ordinary rejection reasons.

## Verification

### Unit and integration tests

- Router dispatch returns while a worker is deliberately blocked.
- `send` creates no completion response and returns after enqueueing.
- `apply` waits for exactly the workers affected by the request.
- A batch affecting two of eight workers waits for two responses.
- Two concurrent `apply` calls receive only their own responses.
- One worker sends one response for a complete input batch.
- Successful apply returns an empty rejection vector.
- Horizon and memory rejections contain only event ID and reason.
- Identical `(event_id, reason)` pairs from multiple workers are deduplicated.
- Different reasons for one event remain separate.
- Queries preserve request order while aggregation remains in `Contime`.
- Inspection merging remains deterministic while aggregation remains in
  `Contime`.
- Advancement waits in `Contime`, and router broadcast does not wait.
- Generated single-target and multi-target route extraction remains correct
  without `Vec<u128>` route results.

### Benchmarks

The benchmark suite separates:

1. Router-only partition preparation for 1, 100, and 1,000 single-target inputs.
2. One worker-message enqueue without waiting.
3. API request-channel creation and synchronous response aggregation.
4. Worker processing independently.
5. Full synchronous `apply`.

Fixtures are created outside timed regions. Router measurements record both
latency and allocation behavior. The structural performance gate is that route
extraction creates no per-input heap objects and router allocations belong to
request-level worker batches rather than individual events.

The existing end-to-end numbers remain diagnostic evidence, not acceptance
targets for this boundary pass. Worker and history performance are optimized in
the subsequent worker-focused pass after the router/API benchmark is isolated.

## Scope

Included:

- Router responsibility reduction.
- Request-scoped synchronous response channels in `Contime`.
- Fire-and-forget `send`.
- Direct worker-to-API responses.
- Direct rejection-vector return type.
- Allocation-free snapshot route extraction.
- Moving query, inspection, advancement, and apply waiting into the API.
- Minimal worker message/admission changes required by the boundary.
- Focused tests and benchmarks.

Deferred:

- Worker journal representation and performance redesign.
- Worker input regrouping optimization.
- History bulk-admission optimization.
- Replay/checkpoint optimization.
- Snapshot hash caching.
- Timeless Runtime integration and benchmarks.
