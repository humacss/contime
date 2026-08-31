# Query Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add fast asynchronous and synchronous snapshot and event-history queries across every isolated ConTime subcrate and the composed core pipeline.

**Architecture:** API, router, and worker keep one unified process queue each and expose ownership-generic query traits. Core supplies concrete apply/snapshot-query/event-query enums implementing adjacent crate traits. Snapshot queries reconstruct a boxed value from a cloned checkpoint plus canonical events without mutating retained state; event queries clone tracked Arc handles into an owned result vector before crossing threads.

**Tech Stack:** Rust 2021, crossbeam-channel 0.5, Criterion 0.5, ahash, existing isolated `contime-*` subcrates.

**Spec:** `docs/superpowers/specs/2026-09-01-query-pipeline-design.md`

## Global Constraints

- Keep every subcrate isolated from the root `contime` crate and from sibling subcrates unless that dependency already exists.
- Keep one unified router queue and one unified worker queue; do not add query-specific process threads or queues.
- Use static dispatch and concrete generic enums only; do not add trait objects, `Any`, or runtime type registries.
- Snapshot queries include the complete event bucket at the requested time and return only found `Box<S>` values.
- Event queries target one snapshot history and return canonical cloned handles over `[from, to)`.
- Worker-local event iterators must not cross a crate boundary, channel, or thread.
- Receiver disconnection is completion; do not introduce completion counts or acknowledgement messages.
- Query replay must not mutate checkpoints, acknowledge history, or alter replay scheduling.
- Preserve all unrelated working-tree changes. Before each commit, inspect the staged file list and stage only task-owned paths.

## File Map

- `crates/events/src/query.rs`: canonical range collection into owned cloned events.
- `crates/events/src/iteration.rs`: reusable inclusive lower-bound iterator construction.
- `crates/events/benches/events.rs`: event-range size and storage-shape matrix.
- `crates/checkpoints/src/query.rs`: read-only historical snapshot reconstruction.
- `crates/checkpoints/benches/query.rs`: checkpoint-hit and replay-depth matrix.
- `crates/api/src/send_query_at.rs`: asynchronous snapshot-query enqueue.
- `crates/api/src/query_at.rs`: synchronous snapshot-query collection.
- `crates/api/src/send_query_events_between.rs`: asynchronous event-query enqueue.
- `crates/api/src/query_events_between.rs`: synchronous event-query collection.
- `crates/api/benches/query.rs`: async enqueue and synchronous collection matrix.
- `crates/router/src/query.rs`: snapshot partitioning and single-history event routing.
- `crates/router/src/route.rs`: unified apply/query receive loop.
- `crates/router/benches/query.rs`: query-count and worker-count routing matrix.
- `crates/worker/src/query.rs`: worker-local snapshot and event query execution.
- `crates/worker/src/work.rs`: unified apply/query receive loop without changing scheduling policy.
- `crates/worker/benches/query.rs`: worker-local found/missing/range query matrix.
- `crates/core/src/query.rs`: public core query methods and synchronous wrappers.
- `crates/core/src/types.rs`: concrete router/worker message enums and query handles.
- `crates/core/src/message.rs`: zero-cost adapters between subcrate contracts.
- `crates/core/src/router.rs`, `crates/core/src/worker.rs`: bind expanded messages to existing runtime processes.
- `crates/core/tests/query.rs`: complete query behavior through the public API.
- `crates/core/benches/query.rs`: end-to-end query throughput and topology matrix.

---

### Task 1: Canonical event-range cloning

**Files:**
- Create: `crates/events/src/query.rs`
- Modify: `crates/events/src/iteration.rs`
- Modify: `crates/events/src/lib.rs`
- Modify: `crates/events/benches/events.rs`
- Modify: `crates/events/README.md`

**Interfaces:**
- Consumes: `EventHistory<E>`, `EventKey<E::Time>`, and the existing merged ordered/late iterator.
- Produces: `EventHistory::clone_between(&self, from: &E::Time, to: &E::Time) -> Vec<E> where E: Clone`.

- [ ] **Step 1: Write failing unit tests for canonical owned results**

Add tests in `query.rs` covering ordered storage, late storage, exact lower-bound inclusion, upper-bound exclusion, empty/missing ranges, and clone independence:

```rust
#[test]
fn clone_between_returns_owned_events_in_canonical_half_open_order() {
    let mut history = EventHistory::new();
    history.insert(event(30, 30));
    history.insert(event(10, 10));
    history.insert(event(20, 20));

    let result = history.clone_between(&10, &30);

    assert_eq!(result.iter().map(|event| event.id).collect::<Vec<_>>(), vec![10, 20]);
}

#[test]
fn cloned_results_survive_history_mutation_and_drop() {
    let mut history = EventHistory::new();
    history.insert(shared_event(1, 10));
    let result = history.clone_between(&0, &20);
    history.insert(shared_event(2, 15));
    drop(history);
    assert_eq!(result[0].event_id(), 1);
}
```

- [ ] **Step 2: Run the focused test and verify the missing method failure**

Run:

```bash
cargo test --manifest-path crates/events/Cargo.toml query::tests
```

Expected: compilation fails because `clone_between` does not exist.

- [ ] **Step 3: Add an inclusive lower-bound iterator constructor**

In `iteration.rs`, add a crate-visible helper that starts at `EventKey { time: from.clone(), event_id: u128::MIN }` across both storage structures. Reuse `EventHistoryRangeIter`; do not allocate or clone here.

```rust
pub(crate) fn iter_from_time(&self, from: &E::Time) -> EventHistoryRangeIter<'_, E> {
    let boundary = EventKey { time: from.clone(), event_id: u128::MIN };
    let ordered_start = self.ordered.partition_point(|(key, _)| key < &boundary);
    EventHistoryRangeIter::new(
        self.ordered.range(ordered_start..),
        self.late.range(boundary..),
    )
}
```

- [ ] **Step 4: Implement owned range collection**

In `query.rs`, stop iteration as soon as canonical time reaches `to` and clone only the stored value:

```rust
pub fn clone_between(&self, from: &E::Time, to: &E::Time) -> Vec<E>
where
    E: Clone,
{
    if from >= to {
        return Vec::new();
    }
    self.iter_from_time(from)
        .take_while(|(key, _)| &key.time < to)
        .map(|(_, event)| event.clone())
        .collect()
}
```

- [ ] **Step 5: Add unit and boundary integration benchmarks**

Benchmark `events/query/clone_between/1000_events` using `Arc<TestEvent>` so the measured operation represents core's eventual tracked-pointer clone path. Prepare history and bounds outside the timed body.

Extend `benches/events.rs` with 0, 10, 100, and 1,000 returned handles over ordered, 10%-late, and 50%-late storage. Use identical event values across storage-shape cases.

- [ ] **Step 6: Run focused verification**

```bash
cargo test --manifest-path crates/events/Cargo.toml
cargo check --all-targets --manifest-path crates/events/Cargo.toml
cargo test --release --manifest-path crates/events/Cargo.toml query::tests::benchmark_query -- --ignored --nocapture
cargo bench --manifest-path crates/events/Cargo.toml --bench events -- query
cargo fmt --manifest-path crates/events/Cargo.toml --check
```

Expected: all tests/checks pass and Criterion reports the 1,000-handle collection cost.

- [ ] **Step 7: Commit only the events query unit**

```bash
git add crates/events/src/query.rs crates/events/src/iteration.rs crates/events/src/lib.rs crates/events/benches/events.rs crates/events/README.md
git diff --cached --name-only
git commit -m "Add canonical event range queries"
```

### Task 2: Read-only checkpoint snapshot queries

**Files:**
- Create: `crates/checkpoints/src/query.rs`
- Create: `crates/checkpoints/benches/query.rs`
- Modify: `crates/checkpoints/Cargo.toml`
- Modify: `crates/checkpoints/src/lib.rs`
- Modify: `crates/checkpoints/README.md`

**Interfaces:**
- Consumes: `CheckpointStore<S>`, `Events`, `ApplyEvents`, `ApplyWrapper`, and canonical `EventRef` iteration.
- Produces: `query_at(&CheckpointStore<S>, &E, &mut W, S::Time) -> Option<Box<S>>` without retained mutation.

- [ ] **Step 1: Write failing query reconstruction tests**

Cover exact checkpoint hits, replay after the nearest checkpoint, initialization without a checkpoint, missing applicable events, complete same-time buckets, and unchanged retained checkpoint keys/snapshots:

```rust
#[test]
fn query_clones_the_nearest_checkpoint_and_applies_through_the_requested_bucket() {
    let (store, events) = fixture_with_checkpoints_at([10, 20], events_at([10, 15, 20, 25]));
    let before = retained_checkpoint_values(&store);

    let result = query_at(&store, &events, &mut (), 20).unwrap();

    assert_eq!(result.time, 20);
    assert_eq!(retained_checkpoint_values(&store), before);
}
```

- [ ] **Step 2: Run the focused test and verify failure**

```bash
cargo test --manifest-path crates/checkpoints/Cargo.toml query::tests
```

Expected: compilation fails because `query_at` is absent.

- [ ] **Step 3: Implement a read-only replay session**

Select the last checkpoint whose key is at or before `CheckpointKey { time, event_id: u128::MAX }`. Clone its snapshot and begin iteration strictly after its exact key. Without a checkpoint, begin unbounded and initialize through `S::create` from the first event whose time is not after the query time.

Group equal timestamps exactly as `replay` does, call the supplied wrapper through `ApplyInner`, and stop before the first event after the query time. Do not call `Events::acknowledge_replay` and do not write to `CheckpointStore`.

- [ ] **Step 4: Return query-local boxed ownership**

Finish with:

```rust
working_snapshot.map(Box::new)
```

Ensure `set_time` is called after each applied bucket and that a checkpoint exactly at the requested time requires no additional application.

- [ ] **Step 5: Add focused unit and integration benchmarks**

Add three benchmarks in `query.rs`:

```text
checkpoints/query/exact_checkpoint
checkpoints/query/replay_10_events
checkpoints/query/replay_1000_events
```

Keep store construction and event preparation outside each timed body; only query-local clone/replay belongs inside.

Create `benches/query.rs` with identical snapshot/event fixtures for checkpoint hits and replay depths of 10, 100, and 1,000 events. Register it as a harness-free bench in `Cargo.toml` and document total and per-event replay cost.

- [ ] **Step 6: Run focused verification**

```bash
cargo test --manifest-path crates/checkpoints/Cargo.toml
cargo check --all-targets --manifest-path crates/checkpoints/Cargo.toml
cargo test --release --manifest-path crates/checkpoints/Cargo.toml query::tests::benchmark_query -- --ignored --nocapture
cargo bench --manifest-path crates/checkpoints/Cargo.toml --bench query
cargo fmt --manifest-path crates/checkpoints/Cargo.toml --check
```

- [ ] **Step 7: Commit only checkpoint querying**

```bash
git add crates/checkpoints/src/query.rs crates/checkpoints/benches/query.rs crates/checkpoints/Cargo.toml crates/checkpoints/src/lib.rs crates/checkpoints/README.md
git diff --cached --name-only
git commit -m "Add read-only checkpoint queries"
```

### Task 3: Isolated API query functions

**Files:**
- Create: `crates/api/src/send_query_at.rs`
- Create: `crates/api/src/query_at.rs`
- Create: `crates/api/src/send_query_events_between.rs`
- Create: `crates/api/src/query_events_between.rs`
- Create: `crates/api/benches/query.rs`
- Modify: `crates/api/Cargo.toml`
- Modify: `crates/api/src/types.rs`
- Modify: `crates/api/src/lib.rs`
- Modify: `crates/api/README.md`

**Interfaces:**
- Consumes: an opaque downstream `Sender<O>` and caller-owned response senders.
- Produces: `SnapshotQueryOutput`, `EventQueryOutput`, the four public query functions, and synchronous receiver-closure collection.

- [ ] **Step 1: Define generic output contracts in `types.rs`**

```rust
pub trait SnapshotQueryOutput<T, S>: Sized {
    fn snapshot_query(time: T, snapshot_ids: Vec<u128>, response: Sender<Vec<Box<S>>>) -> Self;
}

pub trait EventQueryOutput<T, E>: Sized {
    fn event_query(snapshot_id: u128, from: T, to: T, response: Sender<Vec<E>>) -> Self;
}
```

Keep API unaware of routers, workers, snapshots, tracked pointers, and concrete message enums.

- [ ] **Step 2: Write failing async snapshot-query tests**

Use a local adapter output implementing `SnapshotQueryOutput`. Verify IDs/time/sender are forwarded unchanged, empty ID collections close the response channel without forwarding, and a closed output returns `ApiError::OutputChannelClosed`.

- [ ] **Step 3: Implement `send_query_at` minimally**

Collect IDs once. If empty, drop the response sender and return `Ok(())`; otherwise construct `O::snapshot_query(...)` and send it once.

- [ ] **Step 4: Write and implement synchronous snapshot-query tests**

Use a file-local `Deps` trait to stub `send_query_at`, matching the existing apply testing pattern. Verify it flattens all worker vectors until sender closure and propagates enqueue errors.

- [ ] **Step 5: Write and implement event-query counterparts**

`send_query_events_between` forwards one request without interpreting bounds. `query_events_between` owns an unbounded channel, invokes the async dependency, and flattens until closure. The events crate/worker owns `from >= to` semantics.

- [ ] **Step 6: Add unit and boundary integration benchmarks**

Measure message construction/enqueue for async functions and channel collection for synchronous functions. Stub downstream behavior so API unit benchmarks do not execute router or worker logic.

Create `benches/query.rs` comparing async enqueue and synchronous collection for 1, 10, 100, and 1,000 snapshot results and event handles. Register the bench in `Cargo.toml`; keep downstream fixtures deterministic and outside timed bodies.

- [ ] **Step 7: Run focused verification**

```bash
cargo test --manifest-path crates/api/Cargo.toml
cargo check --all-targets --manifest-path crates/api/Cargo.toml
cargo bench --manifest-path crates/api/Cargo.toml --bench query
cargo fmt --manifest-path crates/api/Cargo.toml --check
```

- [ ] **Step 8: Commit only API querying**

```bash
git add crates/api/src/send_query_at.rs crates/api/src/query_at.rs crates/api/src/send_query_events_between.rs crates/api/src/query_events_between.rs crates/api/src/types.rs crates/api/src/lib.rs crates/api/benches/query.rs crates/api/Cargo.toml crates/api/README.md
git diff --cached --name-only
git commit -m "Add isolated query API contracts"
```

### Task 4: Router query partitioning on one unified queue

**Files:**
- Create: `crates/router/src/query.rs`
- Create: `crates/router/benches/query.rs`
- Modify: `crates/router/Cargo.toml`
- Modify: `crates/router/src/types.rs`
- Modify: `crates/router/src/route.rs`
- Modify: `crates/router/src/lib.rs`
- Modify: `crates/router/README.md`

**Interfaces:**
- Consumes: one generic router message implementing `RouteInput`.
- Produces: one apply or query worker message through caller-selected `WorkerOutput` traits.

- [ ] **Step 1: Define zero-cost unified routing contracts**

Add caller-owned variants without importing API or core:

```rust
pub enum RouteInputKind<A, SQ, EQ> {
    Apply(A),
    SnapshotQuery(SQ),
    EventQuery(EQ),
}

pub trait RouteInput {
    type Apply: RouteInputBatch;
    type SnapshotQuery: SnapshotQueryInput;
    type EventQuery: EventQueryInput;
    fn into_kind(self) -> RouteInputKind<Self::Apply, Self::SnapshotQuery, Self::EventQuery>;
}
```

Define query input traits exposing owned parts and query worker-output traits with `create(...)` constructors. Response sender types remain opaque cloneable associated types.

- [ ] **Step 2: Write failing snapshot-partition tests**

Verify each ID is hashed once, requests are grouped into one message per affected worker, the response handle is cloned only for additional workers, empty requests send nothing, and repeated IDs are preserved.

- [ ] **Step 3: Implement snapshot partitioning in `query.rs`**

Allocate one optional ID vector per worker, distribute IDs using the existing `RouterHasher`, then move the response handle into the final affected worker and clone it for preceding workers using the same final-owner pattern as apply routing.

- [ ] **Step 4: Write and implement single-history event routing**

Hash the one snapshot ID and move the complete request and response handle into exactly one worker message. Do not allocate a per-worker vector.

- [ ] **Step 5: Extend the existing receive loop**

Change `route` to receive `B: RouteInput`, match `RouteInputKind`, and call the existing apply route or new query routes. Preserve the no-worker error and normal input-disconnection behavior.

- [ ] **Step 6: Add unit and boundary integration benchmarks**

Measure 1,000 snapshot IDs over 1 and 8 workers, plus one event query over 8 workers. Construction of test messages and channels stays outside timed bodies.

Create `benches/query.rs` with 1, 10, 100, and 1,000 snapshot IDs across 1, 2, 4, 8, and 10 workers, plus event-query routing across the same worker counts. Register the bench in `Cargo.toml`.

- [ ] **Step 7: Run focused verification**

```bash
cargo test --manifest-path crates/router/Cargo.toml
cargo check --all-targets --manifest-path crates/router/Cargo.toml
cargo bench --manifest-path crates/router/Cargo.toml --bench query
cargo fmt --manifest-path crates/router/Cargo.toml --check
```

- [ ] **Step 8: Commit only router querying**

```bash
git add crates/router/src/query.rs crates/router/src/types.rs crates/router/src/route.rs crates/router/src/lib.rs crates/router/benches/query.rs crates/router/Cargo.toml crates/router/README.md
git diff --cached --name-only
git commit -m "Route snapshot and event queries"
```

### Task 5: Worker-local query execution

**Files:**
- Create: `crates/worker/src/query.rs`
- Create: `crates/worker/benches/query.rs`
- Modify: `crates/worker/Cargo.toml`
- Modify: `crates/worker/src/types.rs`
- Modify: `crates/worker/src/work.rs`
- Modify: `crates/worker/src/lib.rs`
- Modify: `crates/worker/README.md`

**Interfaces:**
- Consumes: one generic `WorkInput` yielding apply, snapshot-query, or event-query variants.
- Produces: worker result batches through generic response-handle traits; adds read-only query methods to `Events` and `Checkpoints` contracts.

- [ ] **Step 1: Extend storage contracts with read-only query methods**

```rust
pub trait Events<I>: Sized {
    type Config;
    type Rejection;
    type Time: Clone + Ord;

    fn create(snapshot_id: u128, config: &Self::Config) -> Self;
    fn insert(&mut self, input: I) -> EventInsert<Self::Rejection>;
    fn clone_between(&self, from: &Self::Time, to: &Self::Time) -> Vec<I>
    where
        I: Clone;
}

pub trait Checkpoints<E>: Sized {
    type Config;
    type Context;
    type Time: Clone + Ord;
    type Snapshot;

    fn create(snapshot_id: u128, config: &Self::Config) -> Self;
    fn update(&mut self, events: &mut E, context: &mut Self::Context);
    fn query_at(&self, events: &E, context: &mut Self::Context, time: Self::Time)
        -> Option<Box<Self::Snapshot>>;
}
```

- [ ] **Step 2: Define unified worker input and response traits**

Add `WorkInputKind<A, SQ, EQ>` and `WorkInput::into_kind`, plus owned-parts traits for both query request types. Response traits each consume themselves when sending a non-empty vector, allowing dropping without sending to signal no results.

- [ ] **Step 3: Write failing worker snapshot-query tests**

Preload two histories, send a multi-ID snapshot query containing one missing ID, and verify one result vector contains only two boxed snapshots. Verify the query does not remove dirty schedule entries or call replay acknowledgement.

- [ ] **Step 4: Implement worker snapshot querying**

Look up each requested ID in `snapshots`; invoke `checkpoints.query_at(&slot.events, context, time.clone())`; collect found boxes; send once only when non-empty; then drop the response handle.

- [ ] **Step 5: Write and implement event-query tests**

Verify found canonical handles, empty interval, missing history, and that returned cloned handles remain alive after the worker history is dropped.

- [ ] **Step 6: Integrate query handling into the receive loop**

Match the unified input kind wherever `recv`/`recv_timeout` yields a message. Apply messages keep their current insertion and replay-budget behavior. Query messages execute immediately against current history without popping, adding, or reprioritizing scheduler entries. Timeout behavior remains unchanged.

- [ ] **Step 7: Add unit and boundary integration benchmarks**

Measure 1,000 missing snapshot IDs, 1,000 checkpoint hits through a stub checkpoint store, and cloning 1,000 event handles through a stub event store. Exclude thread startup and channel construction.

Create `benches/query.rs` measuring 1, 10, 100, and 1,000 found snapshots, the same missing counts, and event ranges returning 0, 10, 100, and 1,000 handles. Register the bench in `Cargo.toml`; preload worker-local stores outside timing.

- [ ] **Step 8: Run focused verification**

```bash
cargo test --manifest-path crates/worker/Cargo.toml
cargo check --all-targets --manifest-path crates/worker/Cargo.toml
cargo bench --manifest-path crates/worker/Cargo.toml --bench query
cargo fmt --manifest-path crates/worker/Cargo.toml --check
```

- [ ] **Step 9: Commit only worker querying**

```bash
git add crates/worker/src/query.rs crates/worker/src/types.rs crates/worker/src/work.rs crates/worker/src/lib.rs crates/worker/benches/query.rs crates/worker/Cargo.toml crates/worker/README.md
git diff --cached --name-only
git commit -m "Execute worker-local queries"
```

### Task 6: Core query adapters and public API

**Files:**
- Create: `crates/core/src/query.rs`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/message.rs`
- Modify: `crates/core/src/history.rs`
- Modify: `crates/core/src/checkpoint.rs`
- Modify: `crates/core/src/router.rs`
- Modify: `crates/core/src/worker.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/README.md`

**Interfaces:**
- Consumes: the query contracts produced by Tasks 1–5.
- Produces: concrete unified router/worker messages plus the four approved `ConTime` query methods.

- [ ] **Step 1: Write compile-failing adapter tests**

Manually construct each API query output, convert it through the router adapter and worker adapter, and assert time, IDs, bounds, sender identity/lifetime, and tracked-event ownership are preserved.

- [ ] **Step 2: Define concrete core message enums**

```rust
pub enum RouterMessage<I, S>
where
    I: Input,
{
    Apply(RouterBatch<I>),
    SnapshotQuery {
        time: I::Time,
        snapshot_ids: Vec<u128>,
        response: Sender<Vec<Box<S>>>,
    },
    EventQuery {
        snapshot_id: u128,
        from: I::Time,
        to: I::Time,
        response: Sender<Vec<TrackedEvent<I>>>,
    },
}
```

Define the analogous `WorkerMessage<I, S>` whose snapshot-query variant holds only IDs assigned to that worker. Keep apply payload structs unchanged and wrap them as enum variants.

- [ ] **Step 3: Implement adjacent boundary traits in `message.rs`**

Implement API output traits and router input traits on `RouterMessage`; implement router worker-output traits and worker input traits on `WorkerMessage`. Constructors only wrap or destructure values—no event conversion, snapshot cloning, hashing, or allocation beyond vectors already owned by the calling subsystem.

- [ ] **Step 4: Connect core history and checkpoint queries**

Implement worker `Events::clone_between` for `History<I>` by delegating to `EventHistory<TrackedEvent<I>>::clone_between`. Implement worker `Checkpoints::query_at` for `CheckpointStorage<S, W>` by delegating to `contime_checkpoints::query_at` with the existing wrapper/context behavior.

- [ ] **Step 5: Update router and worker process bindings**

Change the runtime's generic input types from apply-only batches to `RouterMessage<I, S>` and `WorkerMessage<I, S>`. Runtime code itself remains unchanged. `RouterProcess::run` and `WorkerProcess::run` continue delegating to their isolated crate loops.

- [ ] **Step 6: Write public snapshot-query tests before implementation**

Add tests in `query.rs` for asynchronous sender closure, synchronous found-only results, missing histories, exact-time inclusion, historical reconstruction, duplicate IDs, and multi-worker fan-out.

- [ ] **Step 7: Implement public snapshot query methods**

`send_query_at` delegates to `contime_api::send_query_at`. `query_at` delegates to `contime_api::query_at`. Both use `RouterMessage<I, S>` and return worker batches without positional reconstruction.

- [ ] **Step 8: Write and implement public event-query tests**

Verify `[from, to)` ordering, missing/empty behavior, tracked pointer lifetime after later history mutation, asynchronous closure, and synchronous flattening. Implement both event-query methods through the API crate.

- [ ] **Step 9: Add one inline benchmark for each core query method**

Unit benchmarks isolate core adapter overhead using stub downstream channels. End-to-end thread/process work remains for Task 7.

- [ ] **Step 10: Run focused verification**

```bash
cargo test --manifest-path crates/core/Cargo.toml
cargo check --all-targets --manifest-path crates/core/Cargo.toml
cargo fmt --manifest-path crates/core/Cargo.toml --check
```

- [ ] **Step 11: Commit only the core query implementation**

```bash
git add crates/core/src/query.rs crates/core/src/types.rs crates/core/src/message.rs crates/core/src/history.rs crates/core/src/checkpoint.rs crates/core/src/router.rs crates/core/src/worker.rs crates/core/src/lib.rs crates/core/README.md
git diff --cached --name-only
git commit -m "Connect the core query pipeline"
```

### Task 7: End-to-end query tests and benchmarks

**Files:**
- Create: `crates/core/tests/query.rs`
- Create: `crates/core/benches/query.rs`
- Modify: `crates/core/Cargo.toml`
- Modify: `crates/core/README.md`

**Interfaces:**
- Consumes: the complete public core API from Task 6.
- Produces: behavior verification and reproducible throughput documentation for complete query flows.

- [ ] **Step 1: Add end-to-end integration tests**

Build a manual lane fixture without macros. Apply deterministic events, wait for apply completion, then test:

```rust
let snapshots = contime.query_at(20, [existing_a, missing, existing_b]).unwrap();
assert_eq!(snapshots.len(), 2);

let events = contime.query_events_between(existing_a, 10, 20).unwrap();
assert!(events.iter().all(|event| event.time() >= 10 && event.time() < 20));
```

Also test the caller-owned async response channel closes only after every affected worker has dropped its sender.

- [ ] **Step 2: Run integration tests**

```bash
cargo test --manifest-path crates/core/Cargo.toml --test query
```

Expected: all snapshot/event query scenarios pass.

- [ ] **Step 3: Add snapshot-query Criterion groups**

Use 1,000 requested IDs for easy per-item conversion. Measure:

```text
core/query_snapshot/missing/1000_ids
core/query_snapshot/checkpoint_hit/1000_ids
core/query_snapshot/replay_10/1000_ids
core/query_snapshot/replay_1000/1000_ids
```

Prepare runtime state, query vectors, and response channels outside timed regions. The timed region enqueues one request and drains until closure.

- [ ] **Step 4: Add event-query Criterion groups**

Measure returning 0, 10, 100, and 1,000 tracked handles from one snapshot history. Keep fixture population outside timing and report both total latency and events per second.

- [ ] **Step 5: Add topology groups**

Measure 1 router with 1, 2, 4, and 10 workers, then 2 routers with 10 workers. Assign exactly 1,000 snapshot queries to each worker using IDs discovered through the real seeded router. Use one request-scoped channel and end each sample on receiver closure.

- [ ] **Step 6: Run complete query benchmarks and record point estimates**

```bash
cargo bench --manifest-path crates/core/Cargo.toml --bench query
```

Update `crates/core/README.md` with work included, point estimates, per-result cost, throughput, topology, and the distinction between checkpoint hit and query-local replay.

- [ ] **Step 7: Run final repository-scoped verification**

```bash
cargo test --manifest-path crates/events/Cargo.toml
cargo test --manifest-path crates/checkpoints/Cargo.toml
cargo test --manifest-path crates/api/Cargo.toml
cargo test --manifest-path crates/router/Cargo.toml
cargo test --manifest-path crates/worker/Cargo.toml
cargo test --manifest-path crates/core/Cargo.toml
cargo check --all-targets --manifest-path crates/core/Cargo.toml
cargo fmt --manifest-path crates/events/Cargo.toml --check
cargo fmt --manifest-path crates/checkpoints/Cargo.toml --check
cargo fmt --manifest-path crates/api/Cargo.toml --check
cargo fmt --manifest-path crates/router/Cargo.toml --check
cargo fmt --manifest-path crates/worker/Cargo.toml --check
cargo fmt --manifest-path crates/core/Cargo.toml --check
git diff --check
```

Expected: every focused crate test passes, all core targets compile, formatting is unchanged, and no whitespace errors remain.

- [ ] **Step 8: Commit only query integration artifacts**

```bash
git add crates/core/tests/query.rs crates/core/benches/query.rs crates/core/Cargo.toml crates/core/README.md
git diff --cached --name-only
git commit -m "Benchmark end-to-end queries"
```
