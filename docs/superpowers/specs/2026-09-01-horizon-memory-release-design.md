# Horizon Advancement and Memory Release Design

## Scope

Extend the isolated ConTime subcrates and `contime-core` with monotonic history
horizon advancement. Advancement must preserve already accumulated snapshot
state, release tracked memory for obsolete events and checkpoints, forget
pruned event identities, and reject events that subsequently arrive before the
active horizon. The implementation preserves the root ConTime behavior while
keeping policy local to each worker and maintaining the isolated subcrate
boundaries.

Snapshot-internal compaction, transactional advancement across workers, global
worker ordering, and capacity accounting for internal collections remain out
of scope. The implementation does not add a `Snapshot::compact_before` hook.

## Public API

Core exposes an asynchronous enqueue function and a synchronous convenience
wrapper:

```rust
fn send_advance_to(
    &self,
    time: I::Time,
    completion: Sender<()>,
) -> Result<(), ApiError>;

fn advance_to(&self, time: I::Time) -> Result<(), ApiError>;
```

`send_advance_to` returns after enqueueing the request. `advance_to` creates a
request-scoped completion channel, invokes `send_advance_to`, and waits until
every worker-held sender has been dropped. The channel carries no success
payload; disconnection is the completion signal.

Advancement is monotonic per worker. A request at or before a worker's current
time performs no pruning and completes normally. Workers may process the same
advance request at slightly different wall-clock moments. No global barrier is
required beyond the synchronous caller waiting for all workers to finish.

The worker crate defines the isolated time-arithmetic contract required by
advancement: clone, total ordering, a default zero value, and saturating
subtraction by the configured retention value. Core requires the consumer's
time type to implement that contract on advance-capable pipelines. This keeps
generic and composite timestamps compile-time typed without introducing
dynamic arithmetic.

## Message Flow and Ordering

One advance message enters the shared router queue. The router broadcasts one
worker message to every worker, cloning the request-scoped completion sender
once per worker and then dropping its own handle. Each worker handles the
message inside its ordinary serial receive loop.

Worker-local queue order defines which events exist when advancement is
processed. Events already admitted to a history must be reflected in snapshot
state before that history is pruned when necessary. Events processed after the
worker has advanced are evaluated against the new horizon. The router and API
do not inspect histories or impose ordering across workers.

## Horizon Calculation

Each worker stores its current logical time and active history horizon. For a
new target time it computes:

```text
horizon = target_time.saturating_sub(retention_delta)
```

The configured retention delta is supplied by core through worker
configuration. Pruning uses a strict boundary: events and disposable
checkpoints with times less than the horizon are obsolete, while values exactly
at the horizon remain retained.

A snapshot history first encountered after one or more advances is initialized
with the worker's active horizon. Old events cannot bypass admission merely
because their snapshot ID had no previously retained history.

## Replay Before Pruning

For each worker-local snapshot history:

1. Determine whether the history is currently dirty and inspect its dirty
   timestamp.
2. If the history is dirty and `dirty_time < horizon`, replay it before
   removing anything. This ensures unapplied events that are about to disappear
   are reflected in checkpoint state.
3. If the history is clean, or its `dirty_time >= horizon`, do not force its
   ordinary tip replay. Every unapplied event in the latter case remains
   retained and normal scheduled replay may process it later.
4. Materialize the pre-horizon anchor described below, then prune checkpoints
   and events.

A forced replay uses the existing replay and checkpoint machinery rather than
a horizon-specific apply path. Completion for the advance request occurs only
after the worker has completed all required replay and pruning.

## Checkpoint Retention

The checkpoint store retains one replay anchor representing the complete state
through the final event strictly before the horizon, plus every checkpoint at
or after the horizon. It builds that anchor before event pruning by cloning the
closest earlier checkpoint and applying any following pre-horizon event
buckets. If the closest checkpoint already represents the final pruned event,
it can be retained directly. All older pre-horizon checkpoints are removed.

When a snapshot is first materialized, checkpoint machinery retains its clean
initial state at the default time `0`. That initial checkpoint serves as the
anchor until a later pre-horizon anchor can replace it. Histories that have not
materialized a snapshot do not synthesize one merely because time advances.

The anchor preserves accumulated state after its source events are removed.
No snapshot-internal historical compaction is requested. Dropping obsolete
tracked checkpoint boxes releases their tracked allocation and pointer memory.

Queries remain best-effort after pruning. They choose the most appropriate
retained checkpoint and apply whatever retained events are available. If a
query requests a time older than the retained anchor, it may return that anchor
as the closest reproducible state rather than reporting no snapshot merely
because exact history has expired.

## Event Pruning and Admission

The event-history store owns its active horizon. Advancing it performs two
focused removals:

- remove obsolete entries from the late-event `BTreeMap`;
- when the horizon reaches the ordered `VecDeque`, pop obsolete entries from
  its front.

The operation does not add an explicit collection-capacity shrinking policy.
Removed tree nodes and tracked event values release naturally; deque elements
are removed from the front without requiring a replacement allocation.

Every removed event ID is also removed from the retained-identity index. The
same ID may therefore be admitted again after its previous event has passed
through the horizon. Once a history has advanced, a subsequently arriving
event with `time < horizon` is rejected as `BeforeHistoryHorizon`, even if its
ID was previously forgotten. An event exactly at the horizon remains valid.

## Memory Release

The memory subcrate requires no new mechanism. Core event histories contain
tracked shared events and checkpoint stores contain tracked owned snapshots.
Pruning drops those values through their normal ownership paths:

- every removed event handle releases its tracked pointer size;
- the final event handle releases the underlying event allocation;
- every removed checkpoint releases its independently owned tracked snapshot;
- replay-time checkpoint resizing continues to report its ordinary size delta.

Internal map, tree, set, and deque spare capacity remains covered by the
configured safety buffer in this pass. Advancement reports no separate memory
delta because the shared memory budget is updated by tracked drops.

## Subcrate Responsibilities

- `api`: define isolated asynchronous and synchronous advance entry behavior.
- `router`: broadcast one advance request to every worker while preserving the
  request-scoped completion sender lifetime.
- `runtime`: transport core's expanded opaque router and worker messages
  through the existing generic queues and loops.
- `worker`: own local current time and horizon, decide whether replay is
  required, coordinate replay-before-prune, and complete the request.
- `events`: store the active horizon, prune the tree and deque, forget removed
  IDs, and reject later pre-horizon insertions.
- `checkpoints`: materialize one complete pre-horizon replay anchor and remove
  older checkpoints.
- `core`: configure retention, define concrete message enums and adapters,
  expose the consumer API, and map event-store horizon rejection into the
  public rejection reason.
- `memory`: remain unchanged and account for release through tracked drops.

Each isolated crate continues declaring only its own traits and message types.
Only core implements the adapters that connect adjacent contracts.

## Failure and Completion Behavior

Normal advancement has no per-worker success value. A synchronous request is
complete when all worker completion senders close. An enqueue failure remains
an immediate API error. Unexpected router or worker termination may close the
channel early; runtime shutdown outcomes remain responsible for process-health
reporting.

Advancement itself does not reject retained events. Event rejection occurs
only when a later insertion is older than the worker history's active horizon,
using the existing rejection channel and the `BeforeHistoryHorizon` reason.

## Testing and Benchmarking

Focused unit tests cover:

- strict `< horizon` event and checkpoint pruning;
- pruning from both the late-event tree and ordered deque;
- retained-ID removal and later ID reuse;
- rejection before the horizon and acceptance exactly at it;
- forced replay only when `dirty_time < horizon`;
- anchor materialization across events between the previous checkpoint and the
  horizon;
- one retained pre-horizon checkpoint anchor and the initial time-zero anchor;
- monotonic advancement and repeated/older no-op requests;
- best-effort queries older than the retained anchor;
- tracked memory release for removed events and checkpoints;
- asynchronous dispatch and synchronous sender-closure completion.

Unit benchmarks isolate the new hot operations in their owning source units.
Integration benchmarks measure pruning 1, 100, and 1,000 late-tree events,
pruning into the ordered deque, clean versus forced-replay worker advancement,
and core advancement with one and multiple workers. A 1,000-history core case
reports both advancement throughput and released tracked bytes. Runtime
startup, fixture construction, channel construction, and worker warm-up remain
outside measured regions wherever the measured API does not own them.
