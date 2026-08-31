# Arc-only router boundary

> Historical design: superseded by the ownership-generic router boundary on
> 2026-08-31. The router now accepts `Vec<I>` with `I: RoutableInput + Clone`;
> core selects tracked pointer ownership while benchmarks compare owned and
> shared inputs.

## Goal

Require every event entering `contime-router` to be held in `Arc`. The router
resolves each snapshot route once and sends every worker an independently
owned vector containing only the snapshot IDs and events assigned to it.

## Public data model

The underlying event type implements `RoutableInput`; it does not need to
implement `Clone`. Ownership and fan-out are handled by `Arc`:

```rust
pub struct InputBatch<E, C> {
    pub inputs: Vec<Arc<E>>,
    pub completion: C,
}

pub struct RoutedInput<E> {
    pub snapshot_id: u128,
    pub input: Arc<E>,
}

pub struct WorkerBatch<E, C> {
    pub inputs: Vec<RoutedInput<E>>,
    pub completion: C,
}

pub trait RoutableInput {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}
```

`route` remains generic over the underlying event type and completion handle,
but callers cannot pass owned events directly.

## Routing behavior

For each input Arc, the router visits its snapshot IDs in their existing order.
It hashes each snapshot ID to its worker, except that one configured worker
continues to select index zero without hashing. The router moves the original
Arc into the final route and clones it only for preceding additional routes.
Consequently, an event with one snapshot ID causes no event-Arc clone, while an
event with `n` snapshot IDs causes exactly `n - 1` clones.

Each affected worker receives one vector of `RoutedInput` values. Workers do
not revisit snapshot IDs, hash routes, or receive snapshot IDs assigned to
other workers. Events without snapshot IDs produce no worker input. Completion
and worker-unavailable behavior remain unchanged.

## Excluded alternatives

The router will not retain the original event vector behind a shared Arc or
send event indexes to workers. An indexed route containing a `u128` snapshot
ID occupies the same aligned size as a direct `{ snapshot_id, Arc<E> }` route,
adds worker indirection, retains the original vector, and adds Arc operations
for the common single-route case.

The router will not sort routes or deduplicate snapshot IDs. Direct per-worker
vectors remain the simplest independently owned asynchronous worker messages.

## Verification

Unit tests will establish that:

- one snapshot route moves an Arc without cloning it;
- additional snapshot routes clone the Arc exactly once each;
- each worker receives only its assigned snapshot IDs;
- zero-route, completion, deterministic hashing, unavailable-worker, and
  single-worker behavior remain unchanged.

Integration benchmarks will use Arc events exclusively. They will retain the
32-byte boundary measurement, the enabled 64-byte one- and two-route cases,
the single-worker shortcut measurement, and the isolated `Arc::new`
measurements. Historical owned-event results may remain documented as the
evidence for choosing the Arc-only boundary, but owned routing will no longer
be part of the executable benchmark API.

## Scope

This pass changes only the isolated `crates/router` crate. It does not connect
the router to the root `contime` crate, modify workers, or perform git
operations.
