# Timestamped Snapshot Listener Collections Design

## Goal

Add an asynchronous ConTime listener mode that lets a consumer register one
notification collection for a timestamp and a set of snapshot IDs. Each worker
sends at most one notification to that collection after one worker replay batch,
containing every registered snapshot whose state at the watched timestamp may
have changed.

## Semantics

A registration watches the observable state of its snapshots at one timestamp
`T`. A replay affects that observation when the snapshot's conservative replay
boundary is at or before `T`.

```text
replay affected interval: [affected_from, infinity)
listener matches when:    affected_from <= watched_time
```

The boundary is conservative. ConTime reports that the queried state may have
changed; it does not compare materialized snapshot values to prove that it did.
Rejected inputs and retained duplicate IDs do not dirty history and therefore
do not notify listeners. An accepted input after the watched timestamp does not
notify that collection.

One registration call defines one collection. Snapshot IDs within that call are
treated as a set. Repeating an ID in the same call does not create duplicate
notifications, while making two registration calls creates two independent
collections. A listener may be registered before event history exists.

## Public Contract

Core exposes timestamped, batched notifications:

```rust
pub enum SnapshotListenerMessage<T> {
    Registered {
        time: T,
        snapshot_ids: Vec<u128>,
    },
    Replayed {
        time: T,
        snapshot_ids: Vec<u128>,
    },
}
```

and one asynchronous method:

```rust
pub fn send_listen_snapshots(
    &self,
    time: I::Time,
    snapshot_ids: impl IntoIterator<Item = u128>,
    notifications: crossbeam_channel::Sender<SnapshotListenerMessage<I::Time>>,
) -> Result<(), contime_api::ApiError>;
```

The consumer creates and owns the channel. There is no synchronous convenience
function and no explicit unsubscribe command. The same sender may be used for
several collections; the timestamp and returned snapshot IDs identify each
notification. Dropping every receiver makes later sends fail and causes worker
collections to be removed lazily.

## Worker Batch Boundary

The router partitions one registration by worker. Each resulting worker-local
registration is one collection. Consequently, a registration spanning four
workers can produce at most four messages for one replay pass: one from each
affected worker.

A notification is flushed after an actual worker replay batch, not merely after
input insertion. The current replay batches are:

- the replay-budget pass following one received apply batch;
- one overdue replay pass after a deadline timeout; and
- the final replay-all pass during worker shutdown.

Only snapshots that completed replay in that pass are included. If the replay
budget defers part of an input batch, the completed subset is sent now and the
remaining subset is sent in a later notification. If several applies coalesce
before replay, their completed snapshots may naturally share one notification.

## Per-Snapshot Replay Result

The concrete checkpoint adapter captures the event history's dirty timestamp
immediately before checkpoint replay and returns it from `Checkpoints::update`.
The worker wraps that timestamp with the snapshot ID and returns it from
`update_snapshot` after replay completes. Because update is called only for a
dirty scheduled snapshot, no optional result is required.

```rust
pub struct ReplayUpdate<T> {
    pub snapshot_id: u128,
    pub affected_from: T,
}
```

The result is per snapshot. Collapsing an entire worker batch to its earliest
timestamp would cause false notifications for unrelated snapshots.

## Worker-Local Storage

Listener membership is stored directly on the worker's snapshot slot, not in
the consumer snapshot value or checkpoint state:

```text
SnapshotSlot
  optional event history
  optional checkpoints
  request waiters
  notification collection IDs
```

A registration can therefore create a metadata-only slot. The first real event
initializes its event history; listener registration alone does not initialize
history, checkpoints, or consumer state.

Each worker stores collection state once in an indexed arena:

```text
NotificationCollection
  watched time
  notification sender
  pending snapshot IDs
```

Snapshot slots retain only compact collection IDs. Free collection indexes are
reused after disconnection so repeated registration and removal do not grow the
arena without bound.

## Replay Collection Algorithm

For every completed `ReplayUpdate`:

1. Read the notification IDs already stored on that snapshot slot.
2. For each active collection, compare `affected_from` with its watched time.
3. Append the snapshot ID to matching collections and mark each collection as
   touched once for the current replay batch.
4. After the replay batch finishes, send one `Replayed` message for every
   touched collection using its accumulated snapshot IDs.
5. If a send fails, remove the collection and lazily discard its stale IDs from
   snapshot slots when those slots are visited again.

No listener-set intersection, global listener scan, or sender clone per
snapshot occurs on the replay path. Work is proportional to replayed snapshots
plus the notification memberships attached to those snapshots.

## Isolated Crate Boundaries

- **API** collects one timestamp, snapshot-ID collection, and consumer sender
  into one caller-selected output message.
- **Router** partitions the collection by worker and forwards at most one
  registration message per affected worker.
- **Worker** owns metadata-only snapshot slots, collection storage, replay
  matching, batching, and lazy disconnection cleanup.
- **Core** defines concrete messages and adapters between the independently
  declared traits.
- **Runtime, events, checkpoints, lanes, and the legacy root crate** remain
  unchanged. Core's worker-checkpoint adapter derives `affected_from` from the
  existing event-history dirty-time contract immediately before replay.

## Testing

Focused unit tests cover:

- timestamp and collection forwarding at the API boundary;
- deterministic per-worker partitioning at the router boundary;
- registration before event history exists;
- one collection ID stored on every registered snapshot slot;
- one batched registration acknowledgement per worker collection;
- per-snapshot `affected_from` returned after replay;
- notification for accepted events at or before the watched time;
- no notification for later, duplicate, or rejected events;
- one replay message containing every matching snapshot in a worker replay
  batch;
- deferred snapshots appearing in a later replay message;
- independent collections sharing one sender;
- lazy cleanup and arena-index reuse after receiver disconnection.

## Benchmarks

Unit benchmarks measure registration, the no-listener replay fast path,
timestamp filtering, collection accumulation, and batched sends for one, 100,
and 1,000 snapshot IDs.

Core integration benchmarks use a long-lived warmed runtime. They run identical
asynchronous replay workloads with listeners disabled and enabled, and report
the difference as listener overhead. The measured workloads use sustained
batches rather than creating a fresh parked runtime for every iteration.
Topology cases include one router/worker and multiple workers. A registration
spanning several workers expects one notification per affected worker replay
batch, not one global notification.

Cold parked-thread wake latency is not reported as listener throughput. If kept
for diagnostic value, it is labeled separately from steady-state measurements.

## Deferred Issues

- Listener storage is not included in retained event/checkpoint memory
  accounting in this pass.
- There is no explicit unsubscribe command or externally visible collection ID.
- Failed senders and their snapshot-slot memberships are removed lazily.
- More aggressive worker-side coalescing across replay batches remains a later
  optimization.
