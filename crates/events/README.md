# contime-events

`contime-events` is an isolated, passive event-history store. It owns no
threads, channels, snapshots, checkpoints, worker scheduling, memory limits,
or ConTime orchestration.

Core adapts this workspace member into each snapshot history and connects it
to checkpoint storage. The event store itself remains independent of those
crates and knows nothing about their execution topology.

## API guide

Implement [`Event`](https://github.com/humacss/contime/blob/main/crates/events/src/types.rs) for the owned event value or shared handle you
want to store. Its time and ID must stay stable while retained. Create an
[`EventHistory`](https://github.com/humacss/contime/blob/main/crates/events/src/history.rs) with `new`, `with_capacity`, or `with_horizon`,
then call `insert`. Only `Insert::Inserted` changes replay dirtiness; events before
the horizon are rejected before duplicate detection.

Use `iter` for a borrowed canonical history, `iter_from_dirty` for whole-timestamp
replay, or `clone_between(from, to)` for owned results over `[from, to)`. Empty
and reversed query ranges return an empty vector. Queries that start before the
horizon return only retained events and do not report missing older history.
`iter_after` excludes one exact `(time, event_id)` key, rather than the entire
timestamp. `prune_before` advances the horizon monotonically and forgets removed
IDs. These operations never apply events to consumer state.

Generate the public Rust API reference from the workspace root:

```sh
cargo doc -p contime-events --no-deps --open
```

## Responsibilities

- Accept any event ownership representation selected by the consumer.
- Deduplicate retained events by event ID, independently of timestamp.
- Append canonically ordered events to a `VecDeque`.
- Store out-of-order events in a `BTreeMap`.
- Iterate both stores as one canonical `(time, event ID)` history.
- Track the earliest timestamp from which checkpoint replay must resume.
- Iterate from that dirty timestamp while retaining its complete timestamp
  bucket.
- Expose basic length, latest-key, and storage-count inspection.

The store also owns a monotonic history horizon. Events strictly before it are
rejected before duplicate detection, while events exactly at it remain valid.
`prune_before` removes obsolete late-tree and ordered-prefix entries and
forgets their IDs, allowing those IDs to be reused by retained-time events.
Checkpoint materialization belongs to the snapshot store.

## Public boundary

Consumers implement the local `Event` trait by providing an ordered time and a
`u128` event ID. The time type's `Default` value is its zero timestamp.
`EventHistory<E>` stores `E` directly as its consumer-selected ownership type
and is the per-snapshot state object. It does not store a snapshot
ID; the worker owns the mapping from snapshot IDs to histories.

`EventHistory::clone_between` returns owned clones in canonical
`(time, event_id)` order over the half-open interval `[from, to)`. The merged
iterator remains a local borrow; every selected value is cloned before the
result can cross a crate or thread boundary. Pointer-backed event types clone
their handles rather than their payloads.

An empty history starts dirty at zero. Its first unique event sets the dirty
timestamp to that event's time, and subsequent unique events may move the
timestamp earlier. Duplicate IDs do not change it. `mark_replayed` moves the
boundary to the latest retained timestamp, conservatively preserving the last
same-timestamp bucket for the next replay. The first unique event accepted
after that acknowledgement starts a new dirty interval at its own timestamp;
this lets downstream consumers distinguish later changes from the conservative
clean-history boundary.

## Benchmarks

The `events` benchmark measures ordered and late insertion, duplicate rejection,
merged iteration, pruning, and owned query results. Fixtures and retained event
payloads are prepared outside timing. Shared-payload fixtures measure handle
ownership rather than payload copying.

```sh
cargo bench -p contime-events --bench events
```
