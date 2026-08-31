# Query Pipeline Design

## Scope

Extend the isolated ConTime subcrates and `contime-core` with snapshot and
event-history queries. Querying must use the same router and worker processes
as apply, remain compile-time typed, and add no query-triggered scheduling,
checkpoint mutation, retained replay, or history acknowledgement. Advancement,
horizon pruning, and query prioritization remain out of scope.

## Public API

Core exposes asynchronous enqueue functions and synchronous convenience
wrappers for two distinct query kinds:

```rust
fn send_query_at(
    &self,
    time: I::Time,
    snapshot_ids: impl IntoIterator<Item = u128>,
    sender: Sender<Vec<Box<S>>>,
) -> Result<(), ApiError>;

fn query_at(
    &self,
    time: I::Time,
    snapshot_ids: impl IntoIterator<Item = u128>,
) -> Result<Vec<Box<S>>, ApiError>;

fn send_query_events_between(
    &self,
    snapshot_id: u128,
    from: I::Time,
    to: I::Time,
    sender: Sender<Vec<TrackedEvent<I>>>,
) -> Result<(), ApiError>;

fn query_events_between(
    &self,
    snapshot_id: u128,
    from: I::Time,
    to: I::Time,
) -> Result<Vec<TrackedEvent<I>>, ApiError>;
```

Snapshot queries return only histories that can materialize a snapshot at the
requested time. Missing histories and histories without an applicable event
emit no value. Snapshots contain their own IDs, so no external ID or positional
wrapper is returned. Results from different workers have arrival order rather
than global request order. Repeated requested IDs remain repeated requests.

Event queries target one snapshot history. They return retained events in
canonical `(time, event_id)` order over the half-open interval `[from, to)`.
Missing histories, empty ranges, and ranges where `from >= to` all return an
empty vector.

The asynchronous functions return after enqueueing. A synchronous function
creates one request-scoped channel, invokes its asynchronous counterpart, and
collects result batches until the receiver disconnects.

## Message Contracts

Each isolated subcrate declares only the input and output traits required at
its own boundary. Core supplies concrete zero-cost enums implementing adjacent
traits. The router and worker each receive one unified message stream with
apply, snapshot-query, and event-query variants. The runtime continues moving
opaque router and worker messages and does not know query semantics.

One query request owns one response sender. The router moves or clones that
sender once per affected worker, then drops its original handle. Each worker
sends at most one non-empty result vector and drops its handle after completing
the query. Receiver disconnection is the only completion signal; there are no
completion counters or acknowledgement messages.

Snapshot queries hash and partition requested snapshot IDs into one message per
affected worker. Event queries contain one snapshot ID and therefore route to
exactly one worker. Each snapshot ID is hashed once by the router.

## Snapshot Materialization

The checkpoint crate owns query-local snapshot reconstruction:

1. Select the closest retained checkpoint at or before the requested time.
2. Clone that checkpoint into a query-local working snapshot.
3. If no checkpoint exists, initialize a snapshot from the earliest applicable
   event at or before the requested time.
4. Apply canonical event buckets after the selected checkpoint through the
   complete bucket at the requested time, inclusively.
5. Return the working snapshot as `Box<S>`.

The query uses the same filter, decoration, and apply behavior as retained
checkpoint replay. It may use the worker-local apply context required by that
behavior, but it does not mutate retained checkpoints, acknowledge event
history, mark a snapshot clean, or change worker scheduling. A checkpoint
exactly at the requested time already represents the complete bucket at that
time and may be returned from its clone without further replay.

## Event Results and Ownership

An event-history iterator is always a short-lived worker-local borrow. It never
crosses a crate boundary, channel, or thread. The events crate collects the
requested canonical range into an owned vector by cloning each stored event
value before the history borrow ends.

Core stores `TrackedEvent<I>` values, so this generic clone becomes a tracked
Arc clone rather than a payload clone. Returned event handles remain valid if
the history is subsequently changed or pruned. Every live returned handle is
included in tracked memory accounting and releases its pointer accounting when
dropped.

Returned boxed snapshots are consumer-owned query outputs rather than retained
ConTime state. Their allocations are not added to the retained-history memory
budget. Callers that require backpressure may supply a bounded response channel.

## Subcrate Responsibilities

- `api`: construct asynchronous query messages; implement synchronous wrappers
  by owning and draining request-scoped channels.
- `router`: partition snapshot queries, route single-history event queries, and
  preserve sender lifetime without interpreting query results.
- `worker`: look up worker-local histories, invoke the appropriate read path,
  send at most one non-empty result vector, and drop the sender.
- `events`: provide canonical range collection that clones generic stored event
  values into an owned vector.
- `checkpoints`: reconstruct boxed snapshots at a requested time without
  changing retained state.
- `runtime`: remain unchanged apart from accepting core's expanded opaque
  message enums through its existing generic contracts.
- `core`: define concrete router/worker message enums, adapters, public query
  methods, and result ownership types.

## Failure Behavior

The public functions are infallible with respect to missing query data. Their
only immediate error is failure to enqueue into the downstream process. Normal
completion and a query with no results are both represented by response-channel
closure after zero result messages. As with the apply path, unexpected process
termination may close the response channel with only partial results; process
health is reported through runtime shutdown outcomes rather than an additional
query acknowledgement protocol.

## Testing and Benchmarking

Every new executable source unit receives focused unit tests and one inline
Criterion benchmark for its hot operation. Subcrate integration benchmarks
measure complete query work at that boundary without including unrelated
upstream layers.

Core integration tests cover found and missing snapshots, exact-time inclusion,
historical reconstruction, canonical event ranges, duplicate requested IDs,
multi-worker fan-out, asynchronous sender closure, and synchronous collection.
Core integration benchmarks cover checkpoint hits, short and long historical
replay, missing snapshots, small and large event ranges, query batch sizes, and
one/multiple-router and worker topologies. Setup, warm-up, channel construction,
and sender cloning remain outside measured regions wherever the operation under
test does not own them.
