# Timestamped Snapshot Listener Collections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace per-snapshot replay messages with timestamped listener collections that emit one batched notification per collection per worker replay batch.

**Architecture:** One registration carries a watched timestamp, a set of snapshot IDs, and a consumer-owned sender. The router partitions that collection by worker; each worker stores compact collection IDs directly on metadata-capable snapshot slots, derives a per-snapshot `affected_from` timestamp during replay, accumulates matching IDs, and flushes one message per touched collection after the replay pass.

**Tech Stack:** Rust 2021, Crossbeam channels, AHashMap, Criterion.

**Spec:** `docs/superpowers/specs/2026-09-01-snapshot-replay-listeners-design.md`

## Global Constraints

- Keep API, router, and worker crates mutually isolated; Core alone supplies adapters.
- Do not modify runtime, events, checkpoints, lanes, or the legacy root crate.
- Listener registration remains asynchronous and uses a consumer-owned channel.
- Snapshot IDs in one registration are a set; separate calls create independent collections.
- Notify only after completed replay and only when `affected_from <= watched_time`.
- Emit at most one message per touched collection per worker replay batch.
- Store listener metadata on worker-local snapshot slots, never in consumer snapshots or checkpoints.
- Preserve the no-listener replay fast path and avoid sender cloning per snapshot.
- Use test-first red-green-refactor cycles for every behavior.
- Do not commit unless the user explicitly requests it; stop at review checkpoints instead.

## File Structure

- `crates/api/src/send_listen_snapshots.rs`: collect and forward timestamped registrations.
- `crates/api/src/types.rs`: isolated API output trait.
- `crates/router/src/listen.rs`: deterministic registration partitioning.
- `crates/router/src/types.rs`: isolated router input/output traits.
- `crates/worker/src/types.rs`: metadata-capable snapshot slots, replay results, and listener traits.
- `crates/worker/src/checkpoints.rs`: return per-snapshot replay boundaries.
- `crates/worker/src/events.rs`: initialize event history inside metadata-only slots.
- `crates/worker/src/listen.rs`: collection arena, slot memberships, timestamp matching, and batched flush.
- `crates/worker/src/work.rs`: define worker replay batches and flush listener collections.
- `crates/worker/src/query.rs`: skip metadata-only slots during queries.
- `crates/worker/src/advance.rs`: skip metadata-only slots during horizon work.
- `crates/core/src/types.rs`: concrete timestamped public messages and adapters.
- `crates/core/src/message.rs`: API/router/worker trait implementations.
- `crates/core/src/listen.rs`: public `ConTime` method and focused tests.
- `crates/core/tests/listen.rs`: end-to-end correctness.
- `crates/core/benches/listen.rs`: warmed sustained listener-overhead benchmark.

---

### Task 1: Return the affected replay boundary from worker checkpoint updates

**Files:**
- Modify: `crates/worker/src/types.rs`
- Modify: `crates/worker/src/checkpoints.rs`
- Modify: `crates/worker/src/work.rs`

**Interfaces:**
- Produces: `ReplayUpdate<T> { snapshot_id, affected_from }`.
- Consumes: the affected timestamp returned by `Checkpoints::update`.

- [ ] **Step 1: Write the failing checkpoint test**

Add a checkpoint stub whose `update` returns dirty time `37` and assert:

```rust
let update = update_snapshot(7, &mut snapshots, &(), &mut context);
assert_eq!(update, ReplayUpdate { snapshot_id: 7, affected_from: 37 });
assert_eq!(context, vec![1_000]);
```

- [ ] **Step 2: Run the focused test and confirm the missing result fails**

Run:

```bash
cargo test --manifest-path crates/worker/Cargo.toml --lib checkpoints::tests::checkpoint_update_returns_the_affected_interval_start
```

Expected: compilation failure because `update_snapshot` returns `()` and `ReplayUpdate` does not exist.

- [ ] **Step 3: Add the minimal replay result**

Define in `types.rs`:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayUpdate<T> {
    pub snapshot_id: u128,
    pub affected_from: T,
}
```

Change the worker checkpoint contract to return its time:

```rust
fn update(&mut self, events: &mut S, context: &mut Self::Context) -> Self::Time;
```

In `update_snapshot`, receive that value, complete waiters afterward, and return the result:

```rust
let affected_from = checkpoints.update(&mut slot.events, checkpoints_context);
for request in slot.waiters.drain(..) {
    complete_snapshot(request);
}
ReplayUpdate { snapshot_id, affected_from }
```

Update Core's concrete checkpoint adapter to clone `events.dirty_time()` before
calling `contime_checkpoints::replay` and return the captured value afterward.

- [ ] **Step 4: Thread the result through replay callbacks without allocation**

Change `update_budget`, `update_overdue`, and `update_all` callbacks from `FnMut(u128)` to:

```rust
F: FnMut(ReplayUpdate<K::Time>)
```

Pass the concrete result from `update_snapshot`; keep the apply-only `work` path on a no-op callback.

- [ ] **Step 5: Run worker tests**

Run:

```bash
cargo test --manifest-path crates/worker/Cargo.toml --lib checkpoints
cargo test --manifest-path crates/worker/Cargo.toml --lib work
```

Expected: both suites pass without adding a replay-path allocation.

### Task 2: Allow snapshot listener metadata before event history exists

**Files:**
- Modify: `crates/worker/src/types.rs`
- Modify: `crates/worker/src/events.rs`
- Modify: `crates/worker/src/checkpoints.rs`
- Modify: `crates/worker/src/query.rs`
- Modify: `crates/worker/src/advance.rs`

**Interfaces:**
- Produces: `SnapshotSlot` with `events: Option<S>` and `notification_ids: Vec<usize>`.
- Preserves: queries return nothing and horizon advancement performs no history work for metadata-only slots.

- [ ] **Step 1: Write failing metadata-only slot tests**

Cover these cases:

```rust
let slot = SnapshotSlot::<TestEvents, TestCheckpoints, _, ()>::metadata_only();
assert!(slot.events.is_none());
assert!(slot.checkpoints.is_none());
assert!(slot.waiters.is_empty());
assert!(slot.notification_ids.is_empty());
```

Also prove the first routed event initializes `events`, while snapshot queries and advancement ignore a slot that still has no event history.

- [ ] **Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test --manifest-path crates/worker/Cargo.toml --lib events
cargo test --manifest-path crates/worker/Cargo.toml --lib query
cargo test --manifest-path crates/worker/Cargo.toml --lib advance
```

Expected: compilation failures until `SnapshotSlot` supports optional events.

- [ ] **Step 3: Implement the slot representation**

Change the internal type to:

```rust
pub(crate) struct SnapshotSlot<S, K, C, R> {
    pub(crate) events: Option<S>,
    pub(crate) checkpoints: Option<K>,
    pub(crate) waiters: Vec<Request<C, R>>,
    pub(crate) notification_ids: Vec<usize>,
}
```

Add focused constructors for metadata-only and event-initialized slots so call sites do not repeat field initialization.

- [ ] **Step 4: Initialize history only on first real event**

In `insert_event`, create a metadata slot for a vacant entry, then initialize exactly once:

```rust
let events = slot.events.get_or_insert_with(|| S::create(snapshot_id, events_config, horizon));
let result = events.insert(input);
```

Update checkpoint replay to require a dirty slot's event history, and make query/advance paths skip `events: None` rather than materializing anything.

- [ ] **Step 5: Run all worker tests**

Run:

```bash
cargo test --manifest-path crates/worker/Cargo.toml
```

Expected: existing apply, query, scheduling, and horizon behavior remains green.

### Task 3: Replace worker per-snapshot senders with indexed notification collections

**Files:**
- Rewrite: `crates/worker/src/listen.rs`
- Modify: `crates/worker/src/types.rs`
- Modify: `crates/worker/src/work.rs`
- Modify: `crates/worker/src/lib.rs`
- Create: `crates/worker/tests/listener_batches.rs`

**Interfaces:**
- Consumes: `(watched_time, snapshot_ids, listener)` from `SnapshotListenInput`.
- Produces: batched `registered(time, ids)` and `replayed(time, ids)` calls.
- Uses: compact `usize` notification IDs stored in `SnapshotSlot::notification_ids`.

- [ ] **Step 1: Define failing listener collection tests**

Change the isolated traits to:

```rust
pub trait SnapshotListener<T>: Clone {
    fn registered(&self, time: T, snapshot_ids: Vec<u128>) -> bool;
    fn replayed(&self, time: T, snapshot_ids: Vec<u128>) -> bool;
}

pub trait SnapshotListenInput {
    type Time: Clone + Ord;
    type Listener: SnapshotListener<Self::Time>;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener);
}
```

Write tests proving one collection ID is attached to every unique registered slot, duplicate IDs in one call are deduplicated, separate calls remain independent, and one `Registered` batch is emitted.

- [ ] **Step 2: Run the listener tests and verify the old implementation fails**

Run:

```bash
cargo test --manifest-path crates/worker/Cargo.toml --lib listen
```

Expected: compilation and assertion failures from the old per-snapshot listener map.

- [ ] **Step 3: Implement indexed collection storage**

Create internal types in `listen.rs`:

```rust
struct NotificationCollection<T, L> {
    watched_time: T,
    listener: L,
    pending_snapshot_ids: Vec<u128>,
}

pub(crate) struct NotificationCollections<T, L> {
    entries: Vec<Option<NotificationCollection<T, L>>>,
    free: Vec<usize>,
    touched: Vec<usize>,
}
```

Use `free.pop()` before extending `entries`. Sort and deduplicate registration IDs once, send one registration acknowledgement, allocate the collection only if that send succeeds, and attach its index to metadata-only snapshot slots.

- [ ] **Step 4: Implement timestamp matching and batched flush**

For each `ReplayUpdate`, inspect only that slot's `notification_ids`. If the collection is active and `update.affected_from <= collection.watched_time`, append the snapshot ID. Push a collection ID into `touched` only when its pending vector was previously empty.

At the end of the replay pass, drain `touched`, take each pending vector, and call:

```rust
listener.replayed(watched_time.clone(), snapshot_ids)
```

On failure, set the arena entry to `None` and return its index to `free`. Retain only live collection IDs when a snapshot slot is next visited.

- [ ] **Step 5: Flush once at every worker replay boundary**

In `work_messages`, collect during `update_budget`, `update_overdue`, and `update_all`, then call `collections.flush()` exactly once after each helper returns. Do not flush after insertion when no replay completed.

- [ ] **Step 6: Add integration tests through the worker loop**

Cover:

- one apply batch replaying 100 snapshots produces one `Replayed` message containing 100 unique IDs;
- a replay beginning after the watched time produces no message;
- a replay beginning at the watched time does produce a message;
- `replays_per_receive = 1` sends the completed subset and later sends deferred snapshots;
- dropping the receiver reuses the freed collection index on the next registration.

Run:

```bash
cargo test --manifest-path crates/worker/Cargo.toml
```

- [ ] **Step 7: Add focused unit benchmarks**

Benchmark these isolated paths with fixtures outside timing:

- no notification memberships;
- one timestamp comparison that does not match;
- one matching collection;
- one collection accumulating and flushing 100 snapshot IDs;
- one collection accumulating and flushing 1,000 snapshot IDs;
- registration of 1,000 unique snapshot IDs.

### Task 4: Add timestamped listener collections to the isolated API

**Files:**
- Modify: `crates/api/src/send_listen_snapshots.rs`
- Modify: `crates/api/src/types.rs`
- Modify: `crates/api/src/lib.rs`
- Modify: `crates/api/README.md`

**Interfaces:**
- Produces: `SnapshotListenOutput<T, N>::listen(time, snapshot_ids, notifications)`.
- Produces: `send_listen_snapshots(output, time, ids, notifications)`.

- [ ] **Step 1: Update tests first**

Use an output fixture containing `time: u64`, `snapshot_ids`, and the sender. Assert one call forwards time `55`, IDs `[3, 5, 8]`, and the original channel. Retain empty-input and closed-output tests.

- [ ] **Step 2: Run the focused API test and verify failure**

```bash
cargo test --manifest-path crates/api/Cargo.toml --lib send_listen_snapshots
```

- [ ] **Step 3: Implement the timestamped API contract**

Define:

```rust
pub trait SnapshotListenOutput<T, N>: Sized {
    fn listen(time: T, snapshot_ids: Vec<u128>, notifications: Sender<N>) -> Self;
}
```

Collect IDs once. Empty input drops the sender and forwards nothing; non-empty input moves the time, vector, and sender into one output message.

- [ ] **Step 4: Update and run the inline benchmark**

Measure forwarding time plus 1,000 IDs; keep vector/channel construction outside the timed routine. Run the complete API test suite afterward.

### Task 5: Preserve collection boundaries while routing by worker

**Files:**
- Modify: `crates/router/src/listen.rs`
- Modify: `crates/router/src/types.rs`
- Modify: `crates/router/src/route.rs`
- Modify: `crates/router/src/query.rs`
- Modify: `crates/router/src/lib.rs`
- Modify: `crates/router/README.md`

**Interfaces:**
- Consumes: `(time, snapshot_ids, listener)`.
- Produces: at most one timestamped listener collection per affected worker.

- [ ] **Step 1: Update routing tests first**

Assert that time `55` reaches every affected worker, every unique snapshot ID reaches exactly one worker, no worker receives more than one message for the registration, and the listener is cloned once per affected worker rather than per snapshot.

- [ ] **Step 2: Run focused tests and verify failure**

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib listen
cargo test --manifest-path crates/router/Cargo.toml --lib route
```

- [ ] **Step 3: Implement timestamped router traits**

Define:

```rust
pub trait SnapshotListenInput {
    type Time: Clone;
    type Listener: Clone;
    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener);
}

pub trait SnapshotListenWorkerOutput<T, L>: Sized {
    fn listen(time: T, snapshot_ids: Vec<u128>, listener: L) -> Self;
}
```

Clone time and listener once per affected worker and move each original into the final worker message. Keep existing seeded worker selection unchanged.

- [ ] **Step 4: Run router tests and benchmark**

Retain the 1,000-ID, one/eight-worker benchmark and add timestamp forwarding to the fixture without adding work inside the measured route loop.

### Task 6: Adapt the concrete Core API and messages

**Files:**
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/message.rs`
- Modify: `crates/core/src/listen.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/tests/listen.rs`

**Interfaces:**
- Produces: `SnapshotListenerMessage<T>::Registered/Replayed { time, snapshot_ids }`.
- Produces: `ConTime::send_listen_snapshots(time, snapshot_ids, notifications)`.
- Adapts: timestamped API output, router input/output, and worker listener traits.

- [ ] **Step 1: Write failing Core unit tests**

Assert concrete listener callbacks emit:

```rust
SnapshotListenerMessage::Registered {
    time: 55,
    snapshot_ids: vec![3, 5],
}

SnapshotListenerMessage::Replayed {
    time: 55,
    snapshot_ids: vec![3, 5],
}
```

Also assert a dropped receiver makes either callback return `false`.

- [ ] **Step 2: Run focused Core tests and verify failure**

```bash
cargo test --manifest-path crates/core/Cargo.toml --lib listen
```

- [ ] **Step 3: Implement concrete generic messages and adapters**

Change the public enum to:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotListenerMessage<T> {
    Registered { time: T, snapshot_ids: Vec<u128> },
    Replayed { time: T, snapshot_ids: Vec<u128> },
}
```

Make `SnapshotListener<T>` own `Sender<SnapshotListenerMessage<T>>`, add `time` to `SnapshotListen<T>`, and update `RouterMessage` and `WorkerMessage` variants and every isolated trait adapter consistently.

- [ ] **Step 4: Implement the public asynchronous method**

Expose:

```rust
pub fn send_listen_snapshots(
    &self,
    time: I::Time,
    snapshot_ids: impl IntoIterator<Item = u128>,
    notifications: Sender<SnapshotListenerMessage<I::Time>>,
) -> Result<(), ApiError>
```

The method only forwards through the isolated API crate and never waits.

- [ ] **Step 5: Replace the end-to-end correctness test**

With two routers and four workers, register 100 snapshot IDs at time `10`, wait for all worker-local `Registered` batches, asynchronously apply events at time `10`, and assert that flattened `Replayed` batches contain every ID exactly once. Apply events at time `11` and assert no replay notification for the collection watching time `10`.

- [ ] **Step 6: Run all Core tests**

```bash
cargo test --manifest-path crates/core/Cargo.toml
```

### Task 7: Replace cold latency measurements with warmed listener-overhead benchmarks

**Files:**
- Rewrite: `crates/core/benches/listen.rs`
- Modify: `crates/core/README.md`
- Modify: `crates/worker/README.md`
- Modify: `crates/router/README.md`
- Modify: `crates/api/README.md`

**Interfaces:**
- Measures: identical sustained asynchronous replay workloads with listener registration outside timing.
- Reports: baseline, enabled, and calculated listener overhead for each topology and batch size.

- [ ] **Step 1: Build identical baseline and listener fixtures**

For each `(routers, workers)` topology and snapshot count `1`, `100`, and `1_000`, create a long-lived runtime and send one untimed warm-up batch. Register the listener collection outside the timed region for enabled cases. Prepare unique event IDs before timing.

- [ ] **Step 2: Measure sustained worker batches**

Within one Criterion sample, send enough batches to process at least 100,000 total events. Clone one completion sender per input batch, drop the original, and drain the receiver after all sends. Enabled cases additionally drain the expected worker-local replay batches. Divide total duration by events and worker batches.

- [ ] **Step 3: Keep baseline and enabled work identical**

Both cases must use the same event type, timestamps, snapshot IDs, router/worker topology, replay configuration, batch count, and batch size. The only difference is the preinstalled listener collection and notification drain.

- [ ] **Step 4: Run the integration benchmark**

```bash
cargo bench --manifest-path crates/core/Cargo.toml --bench listen
```

Expected output: baseline and enabled point estimates for one router/worker and a multi-worker topology without fresh-runtime latency dominating every iteration.

- [ ] **Step 5: Record the actual listener delta**

In the Core README, report:

```text
listener overhead = enabled duration - baseline duration
```

Keep cold parked-thread latency out of the throughput table. Document it separately only if a dedicated latency benchmark remains.

- [ ] **Step 6: Run unit benchmarks used as sanity checks**

```bash
cargo test --release --manifest-path crates/api/Cargo.toml benchmark_send_listen_snapshots -- --ignored --nocapture
cargo test --release --manifest-path crates/router/Cargo.toml benchmark_snapshot_listener_routing -- --ignored --nocapture
cargo test --release --manifest-path crates/worker/Cargo.toml benchmark_listeners -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml benchmark_listener_notification -- --ignored --nocapture
```

### Task 8: Final verification and documentation audit

**Files:**
- Modify only listener documentation if verification exposes inaccurate wording.

**Interfaces:**
- Verifies: all isolated crates, public Core behavior, and benchmark documentation.

- [ ] **Step 1: Format every touched crate**

```bash
cargo fmt --manifest-path crates/api/Cargo.toml -- --check
cargo fmt --manifest-path crates/router/Cargo.toml -- --check
cargo fmt --manifest-path crates/worker/Cargo.toml -- --check
cargo fmt --manifest-path crates/core/Cargo.toml -- --check
```

- [ ] **Step 2: Run focused complete test suites**

```bash
cargo test --manifest-path crates/api/Cargo.toml
cargo test --manifest-path crates/router/Cargo.toml
cargo test --manifest-path crates/worker/Cargo.toml
cargo test --manifest-path crates/core/Cargo.toml
```

- [ ] **Step 3: Check every target and patch integrity**

```bash
cargo check --all-targets --manifest-path crates/api/Cargo.toml
cargo check --all-targets --manifest-path crates/router/Cargo.toml
cargo check --all-targets --manifest-path crates/worker/Cargo.toml
cargo check --all-targets --manifest-path crates/core/Cargo.toml
git diff --check
```

- [ ] **Step 4: Audit the documented contract**

Confirm the READMEs state all of the following: one timestamp per collection, snapshot IDs treated as a set, one message per touched collection per worker replay batch, per-snapshot conservative time filtering, metadata-only pre-history registration, consumer-owned channels, lazy disconnection cleanup, and listener memory accounting still deferred.

- [ ] **Step 5: Present the uncommitted checkpoint**

Report exact test counts, benchmark point estimates, baseline/enabled deltas, files changed, and remaining deferred issues. Do not commit until the user explicitly requests it.
