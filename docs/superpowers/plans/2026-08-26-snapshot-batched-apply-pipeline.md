# Snapshot-Batched Apply Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace ConTime's expand-and-regroup worker path with API-created snapshot batches that route intact to workers and apply directly to snapshot histories.

**Architecture:** The API groups each request once into ordered `SnapshotInputBatch` values. The router hashes each complete snapshot batch into one worker message, and the worker reserves that complete message before passing each input vector directly to its `SnapshotHistory`. Histories own horizon-scoped input identity; the canonical inspection journal is removed.

**Tech Stack:** Rust 2021, `ahash`, `crossbeam-channel`, Criterion, existing `contime` traits and generated lane enums.

**Spec:** `docs/superpowers/specs/2026-08-26-snapshot-batched-apply-pipeline-design.md`

## Global Constraints

- Keep `apply`, `send`, `query_at`, `advance_to`, constructors, lane traits, derive macros, and snapshot application traits source-compatible except for the approved `ContimeError::MemoryFull` addition.
- Remove `inspect_inputs` and `InputJournalEntry` completely; do not retain a compatibility shim.
- Preserve first-seen snapshot order and caller input order within each snapshot batch.
- The router hashes each distinct snapshot batch once and sends at most one input message per affected worker.
- Snapshot histories, not workers, own duplicate input-ID detection and forget IDs at the history horizon.
- The API memory check is advisory; each worker atomically admits or rejects its complete message.
- `send` is best effort after enqueue and reports only its synchronous API memory precheck.
- Do not introduce dynamic dispatch, trait objects, serialization, or new runtime dependencies.
- Every production behavior change follows RED, GREEN, REFACTOR with the exact focused commands listed below.

---

## File Structure

- Create `src/batch.rs`: ordered API snapshot batching and conservative per-route estimates.
- Create `src/memory.rs`: shared global budget/usage tracker and whole-message reservation operations.
- Create `src/rejection.rs`: `EventRejection` and `EventRejectionReason`, shared by API, worker, and history without layer ownership cycles.
- Modify `src/history/inputs.rs`: horizon-scoped retained-ID set integrated with insertion and pruning.
- Modify `src/history/apply.rs`: routed apply result containing memory delta and history rejections.
- Modify `src/history/storage.rs`: history current-horizon access used by routed admission.
- Modify `src/history/mod.rs`: crate exports for the new history result.
- Modify `src/api.rs`: build snapshot batches, run API memory prechecks, dispatch batches, and remove inspection.
- Modify `src/router/partition.rs`: partition complete snapshot batches rather than individual routes.
- Modify `src/router.rs`: own shared memory tracker, dispatch worker batches, and remove inspection.
- Modify `src/worker.rs`: whole-message reservation and direct per-history batch application.
- Delete `src/worker/admission.rs`: worker-wide identity and per-event reservation disappear.
- Delete `src/journal.rs`: canonical input inspection disappears.
- Modify `src/lib.rs`: module/export cleanup and approved error exports.
- Create `tests/snapshot_batching.rs`: API grouping order and route coverage.
- Modify `tests/edge.rs`, `tests/inputs.rs`, `tests/memory.rs`, `tests/router_api_boundary.rs`, and `tests/apply_boundary_benchmarks.rs`: new identity, failure, and direct-boundary behavior.
- Modify `tests/router_allocations.rs`: request-level allocation and one-message-per-worker coverage for already-grouped snapshot batches.
- Delete `tests/journal.rs`: journal-only public behavior is removed; identity/horizon coverage moves to history and edge tests.
- Modify `tests/public_core_api.rs`: prove inspection symbols are absent.
- Modify `benches/apply.rs`, `benches/apply_boundaries.rs`, `benches/router.rs`, and `benches/helpers.rs`: remove inspection barriers and use snapshot-batched adapters.
- Modify `README.md` and crate docs in `src/lib.rs`: new pipeline, current benchmark results, and provisional memory warning.

---

### Task 1: Move identity and horizon admission into snapshot history

**Files:**
- Create: `src/rejection.rs`
- Modify: `src/lib.rs`
- Modify: `src/api.rs`
- Modify: `src/history/mod.rs`
- Modify: `src/history/inputs.rs`
- Modify: `src/history/apply.rs`
- Modify: `src/history/storage.rs`
- Test: `src/history/inputs.rs`
- Test: `src/history/storage.rs`
- Test: `tests/edge.rs`

**Interfaces:**
- Produces: `EventRejection::new(event_id, reason)` and `EventRejectionReason::{BeforeHistoryHorizon, MemoryFull}` from `src/rejection.rs`.
- Produces: `merge_event_rejections(&mut Vec<EventRejection>, Vec<EventRejection>)` in `src/rejection.rs` for sorted, deduplicated cross-route results.
- Produces: `HistoryApplyResult { pub(crate) bytes_delta: i64, pub(crate) rejections: Vec<EventRejection> }`.
- Produces: `LocalSnapshotHistory::apply_routed_input_batch<C>(&mut self, Vec<S::Input>, &mut C) -> HistoryApplyResult`.
- Preserves: `LocalSnapshotHistory::apply_input_batch<C>(...) -> i64` for focused direct-history callers and benchmarks.

- [ ] **Step 1: Write failing retained-ID and routed-horizon tests**

Add these cases to `src/history/storage.rs` using the existing `apply_one`, `TestEvent`, and `TestSnapshot` fixtures:

```rust
#[test]
fn duplicate_id_at_a_different_time_is_local_to_one_history() {
    let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
    let (mut first, _) = SnapshotHistory::new(snapshot.clone(), 0, 50);
    let (mut second, _) = SnapshotHistory::new(snapshot, 0, 50);

    first.apply_input_batch(vec![TestEvent::Positive(1, 10, 7, 10)], &mut ());
    first.apply_input_batch(vec![TestEvent::Positive(1, 20, 7, 99)], &mut ());
    second.apply_input_batch(vec![TestEvent::Positive(1, 20, 7, 99)], &mut ());

    assert_eq!(first.snapshot_only_at(20).sum, 10);
    assert_eq!(second.snapshot_only_at(20).sum, 99);
}

#[test]
fn pruning_forgets_identity_in_that_history() {
    let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
    let (mut history, _) = SnapshotHistory::new(snapshot, 0, 50);
    history.apply_input_batch(vec![TestEvent::Positive(1, 10, 7, 10)], &mut ());

    history.advance(70);
    history.apply_input_batch(vec![TestEvent::Positive(1, 30, 7, 20)], &mut ());

    assert_eq!(history.snapshot_only_at(70).sum, 30);
}

#[test]
fn routed_apply_rejects_input_before_this_history_horizon() {
    let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
    let (mut history, _) = SnapshotHistory::new(snapshot, 0, 50);
    history.advance(100);

    let result = history.apply_routed_input_batch(vec![TestEvent::Positive(1, 49, 7, 10)], &mut ());

    assert_eq!(result.bytes_delta, 0);
    assert_eq!(result.rejections, vec![EventRejection::new(7, EventRejectionReason::BeforeHistoryHorizon)]);
}
```

In `tests/edge.rs`, remove the `inspect_inputs` assertion from `duplicate_event_id_at_a_different_time_is_a_noop`; its snapshot sum remains the public proof.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test history::storage::tests::duplicate_id_at_a_different_time_is_local_to_one_history -- --nocapture
cargo test history::storage::tests::pruning_forgets_identity_in_that_history -- --nocapture
cargo test history::storage::tests::routed_apply_rejects_input_before_this_history_horizon -- --nocapture
```

Expected: the first two fail because identity is worker-owned or key-scoped; the third fails to compile because `apply_routed_input_batch` and `HistoryApplyResult` do not exist.

- [ ] **Step 3: Move rejection types into `src/rejection.rs`**

Move the unchanged public structs from `src/api.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventRejection {
    pub event_id: u128,
    pub reason: EventRejectionReason,
}

impl EventRejection {
    pub const fn new(event_id: u128, reason: EventRejectionReason) -> Self {
        Self { event_id, reason }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventRejectionReason {
    BeforeHistoryHorizon,
    MemoryFull,
}

pub(crate) fn merge_event_rejections(target: &mut Vec<EventRejection>, incoming: Vec<EventRejection>) {
    if incoming.is_empty() {
        return;
    }
    target.extend(incoming);
    target.sort_unstable();
    target.dedup();
}
```

Declare `mod rejection;` and re-export both public types from `src/lib.rs`. Move the existing merge helper out of `src/api.rs`, import it from `crate::rejection` in API and worker code, and import the public types from `crate` in API, worker, and history code.

- [ ] **Step 4: Make `HistoryInputs` retain IDs with its payloads**

Add `retained_ids: ahash::AHashSet<u128>` to `HistoryInputs`. In `insert_batch_with`, discard an input when `retained_ids.insert(input.id())` returns false before storing it. Count the retained-ID bookkeeping in `HistoryInsert::bytes_delta` using one named constant:

```rust
pub(crate) const RETAINED_ID_BYTES: u64 = (size_of::<u128>() * 2) as u64;
```

When `prune_before` removes an input from either store, remove its ID from `retained_ids` and include `RETAINED_ID_BYTES` in `PrunedInputs::bytes`. Add a private `contains_id(u128) -> bool` used by history admission. Preserve the existing ordered-deque/late-tree invariants.

- [ ] **Step 5: Add checked routed history application**

In `src/history/apply.rs`, add:

```rust
pub(crate) struct HistoryApplyResult {
    pub(crate) bytes_delta: i64,
    pub(crate) rejections: Vec<EventRejection>,
}
```

Implement `apply_routed_input_batch` by filtering retained duplicate IDs as successful no-ops, rejecting non-duplicate inputs whose time is before `current_time - lower_time_horizon_delta`, and passing the remaining owned vector to the existing apply path. Expose a crate-private `earliest_retained_time()` from `LocalSnapshotHistory`; do not expose mutable history internals.

- [ ] **Step 6: Run focused and regression tests and verify GREEN**

Run:

```bash
cargo test history::inputs -- --nocapture
cargo test history::storage -- --nocapture
cargo test --test edge -- --nocapture
cargo test --test history_input_count -- --nocapture
```

Expected: all focused history, edge, and history-input-count tests pass.

- [ ] **Step 7: Commit Task 1**

```bash
git add src/rejection.rs src/lib.rs src/api.rs src/history/mod.rs src/history/inputs.rs src/history/apply.rs src/history/storage.rs tests/edge.rs
git commit -m "refactor: move input identity into snapshot history"
```

---

### Task 2: Build ordered snapshot batches at the API boundary

**Files:**
- Create: `src/batch.rs`
- Modify: `src/lib.rs`
- Create: `tests/snapshot_batching.rs`

**Interfaces:**
- Produces: `SnapshotInputBatch<IL> { pub(crate) snapshot_id: u128, pub(crate) inputs: Vec<IL>, pub(crate) conservative_bytes: u64 }`.
- Produces: `group_inputs_by_snapshot<SL, IL, I>(inputs: I) -> Vec<SnapshotInputBatch<IL>>`.
- Produces: `SnapshotInputBatch::unique_input_ids(&self, target: &mut Vec<u128>)` for rare rejection construction.
- Consumes: `InputLanes<SL>::visit_snapshot_ids` and `Input::conservative_size`.

- [ ] **Step 1: Write failing snapshot-batching tests**

Create `tests/snapshot_batching.rs` with a dynamic marker fixture and generated lanes. Add tests that call a doc-hidden `SnapshotBatchBenchmark::group` adapter and assert these exact projections:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
struct RouteMarker {
    event_id: u128,
    time: i64,
    snapshot_ids: Vec<u128>,
}

impl Input for RouteMarker {
    type Time = i64;

    fn id(&self) -> u128 { self.event_id }
    fn time(&self) -> i64 { self.time }
    fn conservative_size(&self) -> u64 {
        size_of::<Self>() as u64 + (self.snapshot_ids.capacity() * size_of::<u128>()) as u64
    }
}

impl Marker for RouteMarker {}

impl InputRoute for RouteMarker {
    fn visit_snapshot_ids<F>(&self, visit: &mut F)
    where
        F: FnMut(u128),
    {
        self.snapshot_ids.iter().copied().for_each(visit);
    }
}

contime::lanes! {
    mod batching_lanes;
    snapshots [TestSnapshot];
    markers [RouteMarker];
    routes [TestEvent => [TestSnapshot]];
}

fn marker<const N: usize>(event_id: u128, snapshot_ids: [u128; N]) -> batching_lanes::InputLanes {
    RouteMarker { event_id, time: 10, snapshot_ids: snapshot_ids.into_iter().collect() }.into()
}

#[test]
fn api_grouping_preserves_first_snapshot_and_per_snapshot_input_order() {
    let grouped = SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([
        marker(1, [7, 3]),
        marker(2, [3, 9]),
        marker(3, [7]),
    ]);

    assert_eq!(grouped, vec![(7, vec![1, 3]), (3, vec![1, 2]), (9, vec![2])]);
}

#[test]
fn api_grouping_discards_unrouted_inputs() {
    let grouped = SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([
        marker(1, []),
        marker(2, [5]),
    ]);
    assert_eq!(grouped, vec![(5, vec![2])]);
}
```

The adapter must return only `(snapshot_id, input_ids)` projections so tests do not expose production batch internals.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --test snapshot_batching -- --nocapture
```

Expected: compile failure because `SnapshotBatchBenchmark` and API grouping do not exist.

- [ ] **Step 3: Implement `SnapshotInputBatch` and ordered grouping**

In `src/batch.rs`, use an `AHashMap<u128, usize>` for lookup and an ordered `Vec<SnapshotInputBatch<IL>>` for output. Reuse one route scratch vector per request. For an input with `N` routes, clone for routes `0..N-1` and move into route `N-1`. Calculate each routed copy's estimate as:

```rust
fn conservative_route_bytes<I: Input>(input: &I) -> u64 {
    input.conservative_size().saturating_mul(2).saturating_add(RETAINED_ID_BYTES)
}
```

Add the doc-hidden benchmark projection adapter and export it from `src/lib.rs` under `#[doc(hidden)]`.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run:

```bash
cargo test --test snapshot_batching -- --nocapture
cargo test --test inputs marker_route_visitor_preserves_dynamic_order_and_empty_routes -- --nocapture
```

Expected: both tests pass and dynamic route visitation remains unchanged.

- [ ] **Step 5: Commit Task 2**

```bash
git add src/batch.rs src/lib.rs tests/snapshot_batching.rs
git commit -m "feat: group api inputs into snapshot batches"
```

---

### Task 3: Remove retained-input inspection and the worker journal

**Files:**
- Delete: `src/journal.rs`
- Delete: `tests/journal.rs`
- Modify: `src/lib.rs`
- Modify: `src/api.rs`
- Modify: `src/router.rs`
- Modify: `src/worker.rs`
- Modify: `tests/inputs.rs`
- Modify: `tests/fragments.rs`
- Modify: `tests/public_core_api.rs`
- Modify: `benches/apply.rs`
- Modify: `README.md`

**Interfaces:**
- Removes: `Contime::inspect_inputs`, `InputJournalEntry`, `WorkerInbound::InputsInRange`, and `Router::dispatch_inspection`.
- Preserves: replay inputs inside each `SnapshotHistory`; no replay code may read the removed journal.

- [ ] **Step 1: Write the failing public-boundary assertion**

Extend `tests/public_core_api.rs`:

```rust
for forbidden in [
    "pub use journal::InputJournalEntry",
    "pub fn inspect_inputs",
    "InputsInRange",
    "dispatch_inspection",
] {
    assert!(!source.contains(forbidden), "public and worker inspection symbol remained: {forbidden}");
}
```

Read `src/api.rs`, `src/lib.rs`, `src/router.rs`, and `src/worker.rs` into the test rather than checking only `src/lib.rs`.

- [ ] **Step 2: Run the boundary test and verify RED**

Run:

```bash
cargo test --test public_core_api -- --nocapture
```

Expected: failure naming the still-present inspection symbols.

- [ ] **Step 3: Delete the inspection path**

Delete `src/journal.rs` and `tests/journal.rs`. Remove journal exports, `inspect_inputs`, owned range-bound conversion used only by inspection, router inspection dispatch, worker inspection messages, `input_log`, `record_worker_inputs`, `prune_input_log`, and journal memory reconciliation.

In tests that currently use inspection only as an assertion, replace it with the observable snapshot or pending-history behavior already exercised by that test. Delete `input_inspection_returns_one_global_input_with_every_route`; Task 2 owns route projection coverage. Move `horizon_forgets_identity_and_allows_the_id_at_a_new_retained_time` coverage into Task 1 history tests.

In `benches/apply.rs`, replace asynchronous `send` synchronization through `inspect_inputs(..)` with `advance_to(next_time)` before the next timed iteration and a final `advance_to(next_time)` after the loop. Do not include either barrier in the measured duration.

- [ ] **Step 4: Run focused removals and behavior tests and verify GREEN**

Run:

```bash
cargo test --test public_core_api -- --nocapture
cargo test --test inputs -- --nocapture
cargo test --test fragments -- --nocapture
cargo test --test memory -- --nocapture
cargo bench --bench apply --no-run
rg -n "inspect_inputs|InputJournalEntry|InputsInRange|dispatch_inspection|input_log" src benches README.md tests --glob '!public_core_api.rs'
```

Expected: tests and benchmark compilation pass; `rg` returns no matches.

- [ ] **Step 5: Commit Task 3**

```bash
git add -A src/journal.rs tests/journal.rs src/lib.rs src/api.rs src/router.rs src/worker.rs tests/inputs.rs tests/fragments.rs tests/public_core_api.rs benches/apply.rs README.md
git commit -m "refactor: remove retained input inspection"
```

---

### Task 4: Add shared whole-request memory tracking

**Files:**
- Create: `src/memory.rs`
- Modify: `src/lib.rs`
- Modify: `src/router.rs`
- Modify: `src/api.rs`
- Modify: `src/worker.rs`
- Test: `src/memory.rs`

**Interfaces:**
- Produces: cloneable crate-private `MemoryTracker` backed by shared `Arc<AtomicU64>` budget and usage.
- Produces: `remaining() -> u64`, `can_fit(bytes: u64) -> bool`, `try_reserve(bytes: u64) -> bool`, `release(bytes: u64)`, and `apply_delta(delta: i64)`.
- Produces: `reconcile_reservation(reserved: u64, actual_delta: i64)`, which converts one conservative reservation into the exact signed history-memory delta.
- Produces: `Router::memory_tracker() -> MemoryTracker` for `Contime` construction.
- Replaces: raw memory budget and usage atomics passed independently through constructors.

- [ ] **Step 1: Write failing memory-tracker unit tests**

Create these tests in `src/memory.rs`:

```rust
#[test]
fn advisory_check_does_not_reserve_memory() {
    let tracker = MemoryTracker::new(100);
    assert!(tracker.can_fit(80));
    assert_eq!(tracker.remaining(), 100);
}

#[test]
fn whole_message_reservation_is_atomic() {
    let tracker = MemoryTracker::new(100);
    assert!(tracker.try_reserve(80));
    assert!(!tracker.try_reserve(21));
    assert_eq!(tracker.remaining(), 20);
}

#[test]
fn releasing_overestimate_restores_capacity() {
    let tracker = MemoryTracker::new(100);
    assert!(tracker.try_reserve(80));
    tracker.release(30);
    assert_eq!(tracker.remaining(), 50);
}

#[test]
fn reservation_reconciliation_keeps_only_actual_growth() {
    let tracker = MemoryTracker::new(100);
    assert!(tracker.try_reserve(80));
    tracker.reconcile_reservation(80, 30);
    assert_eq!(tracker.remaining(), 70);
}

#[test]
fn negative_actual_delta_releases_reservation_and_existing_usage() {
    let tracker = MemoryTracker::new(100);
    assert!(tracker.try_reserve(40));
    assert!(tracker.try_reserve(20));
    tracker.reconcile_reservation(20, -10);
    assert_eq!(tracker.remaining(), 70);
}
```

- [ ] **Step 2: Run the unit tests and verify RED**

Run:

```bash
cargo test memory::tests -- --nocapture
```

Expected: compile failure because `MemoryTracker` does not exist.

- [ ] **Step 3: Implement the tracker and wire shared ownership**

Implement `try_reserve` with one `fetch_update(Ordering::Relaxed, Ordering::Relaxed, ...)`. Implement signed reconciliation with the existing saturating compare-exchange loop moved out of `worker.rs` into `MemoryTracker::apply_delta`. Implement `reconcile_reservation` exactly as follows: for a nonnegative `actual_delta`, debug-assert that it does not exceed `reserved` and release `reserved - actual_delta`; for a negative delta, release all `reserved` bytes and then pass the negative delta to `apply_delta`. Saturating arithmetic keeps accounting conservative in release builds.

Construct one tracker in `Router::with_history_horizon_and_contexts`, clone it into each worker, and expose a clone to `Contime` during construction. Keep the constructor's `memory_budget_bytes` semantics global.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run:

```bash
cargo test memory::tests -- --nocapture
cargo test --test memory -- --nocapture
```

Expected: tracker tests and existing public memory tests pass before the message-shape migration.

- [ ] **Step 5: Commit Task 4**

```bash
git add src/memory.rs src/lib.rs src/router.rs src/api.rs src/worker.rs
git commit -m "refactor: centralize contime memory tracking"
```

---

### Task 5: Route snapshot batches and apply worker messages directly

**Files:**
- Delete: `src/worker/admission.rs`
- Modify: `src/api.rs`
- Modify: `src/router/partition.rs`
- Modify: `src/router.rs`
- Modify: `src/worker.rs`
- Modify: `src/lib.rs`
- Modify: `tests/snapshot_batching.rs`
- Modify: `tests/router_api_boundary.rs`
- Modify: `tests/memory.rs`
- Modify: `tests/apply_boundary_benchmarks.rs`
- Modify: `tests/router_allocations.rs`

**Interfaces:**
- Consumes: `group_inputs_by_snapshot` and `SnapshotInputBatch<IL>` from Task 2.
- Consumes: `MemoryTracker` from Task 4.
- Consumes: `apply_routed_input_batch(...) -> HistoryApplyResult` from Task 1.
- Produces: `WorkerInbound::Inputs { snapshot_batches, conservative_bytes, completion }`.
- Produces: `RoutePartitioner::partition_snapshot_batches(...) -> Vec<WorkerInputBatch<IL>>`.
- Produces: `memory_full_rejections(&[SnapshotInputBatch<IL>]) -> Vec<EventRejection>`, which collects all input IDs, sorts and deduplicates them, and maps each to `MemoryFull` without journal state.

- [ ] **Step 1: Write failing direct-pipeline tests**

Update `tests/apply_boundary_benchmarks.rs` so adapters accept one prepared `SnapshotInputBatch` rather than raw routed inputs. Add:

```rust
#[test]
fn worker_applies_pre_grouped_snapshot_batches_without_regrouping() {
    let worker = WorkerApplyBenchmark::<TestSnapshotLanes, TestInputLanes>::new(MEMORY_BUDGET_BYTES, 100);
    worker.warm_up(1);
    let batch = worker.prepare_snapshot_batch(7, inputs());

    assert!(worker.apply_snapshot_batches(vec![batch]).is_empty());
    let snapshot: TestSnapshot = worker.snapshot_at(7, 10).unwrap().into();
    assert_eq!(snapshot.sum, 3);
}

#[test]
fn worker_rejects_a_complete_message_before_mutating_any_history() {
    let worker = WorkerApplyBenchmark::<TestSnapshotLanes, TestInputLanes>::new(1, 100);
    worker.warm_up(1);
    let batches = vec![
        worker.prepare_snapshot_batch(7, vec![TestEvent::Positive(7, 10, 11, 1).into()]),
        worker.prepare_snapshot_batch(9, vec![TestEvent::Positive(9, 10, 12, 1).into()]),
    ];

    assert_eq!(
        worker.apply_snapshot_batches(batches),
        vec![
            EventRejection::new(11, EventRejectionReason::MemoryFull),
            EventRejection::new(12, EventRejectionReason::MemoryFull),
        ]
    );
    assert!(worker.snapshot_at(7, 10).is_none());
    assert!(worker.snapshot_at(9, 10).is_none());
}

#[test]
fn router_boundary_reports_the_worker_that_loses_a_shared_reservation() {
    let one_batch_budget = SnapshotBatchBenchmark::total_conservative_bytes::<TestSnapshotLanes, TestInputLanes, _>([
        TestEvent::Positive(1, 10, 11, 1).into(),
    ]);
    let router = RouterApplyBenchmark::<TestSnapshotLanes, TestInputLanes>::new(2, one_batch_budget, 100);
    let [first_snapshot_id, second_snapshot_id] = router.snapshot_ids_on_distinct_workers();
    let inputs = [
        TestEvent::Positive(first_snapshot_id, 10, 11, 1).into(),
        TestEvent::Positive(second_snapshot_id, 10, 12, 1).into(),
    ];

    let rejections = router.apply_snapshot_batches(router.prepare_snapshot_batches(inputs));

    assert_eq!(rejections.len(), 1);
    assert_eq!(rejections[0].reason, EventRejectionReason::MemoryFull);
    let materialized = [
        router.snapshot_at(first_snapshot_id, 10).is_some(),
        router.snapshot_at(second_snapshot_id, 10).is_some(),
    ];
    assert_eq!(materialized.into_iter().filter(|exists| *exists).count(), 1);
}
```

Extend the doc-hidden benchmark adapters only as needed by these tests: `SnapshotBatchBenchmark::total_conservative_bytes` sums the production grouped batches, `RouterApplyBenchmark::prepare_snapshot_batches` calls production API grouping, `apply_snapshot_batches` bypasses only the API precheck and enters production router dispatch, and `snapshot_ids_on_distinct_workers` deterministically scans integer snapshot IDs through that router's own partitioner until it finds one ID per worker. These adapters must not duplicate batching or routing logic.

Do not duplicate the cross-history identity test here: Task 1's exact two-history test proves that the same input ID is independently admissible per snapshot history. Keep `tests/router_api_boundary.rs` focused on request-scoped completions and multi-worker rejection deduplication; the router-boundary test above proves partial worker admission is reported synchronously.

In `tests/memory.rs`, add:

```rust
#[test]
fn api_precheck_rejects_the_complete_apply_request() {
    let contime = TestSnapshotContime::new(1, 1);
    let rejections = contime
        .apply([TestEvent::Positive(1, 10, 10, 1), TestEvent::Positive(1, 10, 20, 1)].map(Into::into))
        .unwrap();
    assert_eq!(rejections, vec![
        EventRejection::new(10, EventRejectionReason::MemoryFull),
        EventRejection::new(20, EventRejectionReason::MemoryFull),
    ]);
    assert!(contime.query_at(10, &[1]).unwrap()[0].is_none());
}

#[test]
fn api_precheck_returns_memory_full_error_for_send() {
    let contime = TestSnapshotContime::new(1, 1);
    assert!(matches!(
        contime.send([TestEvent::Positive(1, 10, 10, 1).into()]),
        Err(ContimeError::MemoryFull)
    ));
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test --test apply_boundary_benchmarks -- --nocapture
cargo test --test router_api_boundary -- --nocapture
cargo test --test memory api_precheck -- --nocapture
```

Expected: compile failures for the new batch adapters and `ContimeError::MemoryFull`, followed by behavioral failures while worker-wide identity and old routed-input dispatch remain.

- [ ] **Step 3: Partition complete snapshot batches**

Replace `RoutePartitioner::partition` with a function that accepts `Vec<SnapshotInputBatch<IL>>`, hashes `batch.snapshot_id` once, and moves the complete batch into that worker bucket. Sum `batch.conservative_bytes` into each `WorkerInputBatch`. Preserve batch order within each bucket.

Update `tests/router_allocations.rs` to construct the 1,000-event snapshot batch before enabling allocation counting, pass that prepared vector to production partitioning, and assert `(affected_workers, snapshot_batches) == (1, 1)`. Keep the request-level allocation ceiling at eight allocations. This simultaneously proves the router sends one complete message to the one affected worker. Update the router Criterion fixture to construct snapshot batches outside its timed partition closure.

- [ ] **Step 4: Replace worker admission and direct-apply snapshot batches**

Delete `src/worker/admission.rs`, `WorkerInput`, worker-wide retained IDs, per-input memory reservation, and `apply_inputs_to_histories` regrouping.

In `src/batch.rs`, implement the rejection helper used by both API precheck and worker rejection:

```rust
pub(crate) fn memory_full_rejections<IL: Input>(batches: &[SnapshotInputBatch<IL>]) -> Vec<EventRejection> {
    let mut ids = batches.iter().flat_map(|batch| batch.inputs.iter().map(Input::id)).collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter().map(|event_id| EventRejection::new(event_id, EventRejectionReason::MemoryFull)).collect()
}
```

For one `WorkerInbound::Inputs` message:

```rust
if !memory.try_reserve(conservative_bytes) {
    complete(completion, memory_full_rejections(&snapshot_batches));
    continue;
}

let mut actual_delta = 0_i64;
let mut rejections = Vec::new();
for batch in snapshot_batches {
    let history = match history_by_id.entry(batch.snapshot_id) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            let (history, base_delta) = SnapshotHistory::new_with_snapshot_id(
                batch.snapshot_id,
                current_time.clone(),
                lower_time_horizon_delta.clone(),
            );
            actual_delta = actual_delta.saturating_add(base_delta);
            entry.insert(history)
        }
    };
    let result = history.apply_routed_input_batch(batch.inputs, &mut apply_context);
    actual_delta = actual_delta.saturating_add(result.bytes_delta);
    merge_event_rejections(&mut rejections, result.rejections);
}
memory.reconcile_reservation(conservative_bytes, actual_delta);
complete(completion, rejections);
```

Include the base delta returned when creating a pending history in `actual_delta`. Assert in debug builds that nonnegative actual growth does not exceed the conservative reservation. Horizon advancement continues to call `memory.apply_delta(bytes_delta)`.

- [ ] **Step 5: Add API precheck and dispatch already-grouped batches**

Store `MemoryTracker` in `Contime`. In both `apply` and `send`, call `group_inputs_by_snapshot` once and sum `conservative_bytes`.

For `apply`, if `!memory.can_fit(total)`, return sorted/deduplicated `MemoryFull` rejections for every routed input ID. For `send`, return `Err(ContimeError::MemoryFull)`. Otherwise dispatch the prepared snapshot batches through the router. Do not reserve in the API.

Add `MemoryFull` to `ContimeError`. Keep request-scoped completion channels for synchronous apply. Worker rejections remain merged by `(event_id, reason)`, so one rejected route reports an incompletely applied event even when another route succeeded.

- [ ] **Step 6: Run focused pipeline and behavior tests and verify GREEN**

Run:

```bash
cargo test --test snapshot_batching -- --nocapture
cargo test --test apply_boundary_benchmarks -- --nocapture
cargo test --test router_api_boundary -- --nocapture
cargo test --test memory -- --nocapture
cargo test --test inputs -- --nocapture
cargo test --test edge -- --nocapture
cargo test --test query -- --nocapture
```

Expected: all focused API, router, worker, memory, identity, replay, and query tests pass.

- [ ] **Step 7: Commit Task 5**

```bash
git add -A src/worker/admission.rs src/api.rs src/router/partition.rs src/router.rs src/worker.rs src/lib.rs tests/snapshot_batching.rs tests/router_api_boundary.rs tests/memory.rs tests/apply_boundary_benchmarks.rs tests/router_allocations.rs
git commit -m "refactor: route snapshot batches directly to histories"
```

---

### Task 6: Refresh benchmarks, documentation, and full verification

**Files:**
- Modify: `benches/apply_boundaries.rs`
- Modify: `benches/router.rs`
- Modify: `benches/helpers.rs`
- Modify: `benches/apply.rs`
- Modify: `README.md`
- Modify: `src/lib.rs`
- Create: `docs/superpowers/reports/2026-08-26-snapshot-batched-apply-pipeline-report.md`

**Interfaces:**
- Consumes: production snapshot batching and worker-message adapters from Task 5.
- Preserves: Criterion group `apply_1000_events_one_snapshot/{api,router,worker,snapshot_history}`.
- Produces: current exact 30-sample intervals and an implementation report tied to the final commit SHA.

- [ ] **Step 1: Update the matched benchmark fixtures**

Make all four rows apply the same 1,000 unique `BenchInputLanes` to snapshot `1` at time `10`. Construct API inputs, snapshot batches, router state, worker state, and histories outside the timed region. Warm worker threads before timing. The measured closures must contain only their named entry boundary.

Keep the history stress groups `history_late_rate`, `history_reverse_batch`, `history_merged_replay`, and `history_horizon_prune`. Delete `benchmark_send_persistent_matrix`, `benchmark_sync_apply_persistent_matrix`, `benchmark_sync_apply_end_to_end`, and their Criterion registrations; the matched four-row stack replaces those overlapping end-to-end measurements.

- [ ] **Step 2: Compile every benchmark target**

Run:

```bash
cargo bench --bench apply_boundaries --no-run
cargo bench --bench apply --no-run
cargo bench --bench router --no-run
```

Expected: all optimized benchmark executables compile without warnings.

- [ ] **Step 3: Run the exact matched benchmark**

Run:

```bash
cargo bench --bench apply_boundaries -- apply_1000_events_one_snapshot --sample-size 30
```

Record Criterion's exact `[low estimate high]` intervals for API, router, worker, and snapshot history. Calculate per-event point estimates by dividing each middle estimate by `1,000`. Calculate adjacent residuals only where confidence intervals make the subtraction meaningful; label overlapping intervals as not separable.

- [ ] **Step 4: Update README and crate documentation**

Replace the current outside-in table with the fresh intervals from Step 3. Explain the measured work at each row and identify the dominant remaining residual without attributing overlapping noise.

Document:

```text
API inputs -> snapshot batches -> worker messages -> snapshot histories
```

Add a clearly labeled provisional-memory section stating that the API check is advisory, concurrent requests may pass together, worker reservations may cause partial cross-worker or cross-snapshot application, synchronous `apply` reports affected IDs, asynchronous `send` is best effort after enqueue, conservative estimates may over-reject, and transactional consistency is deferred.

Remove all inspection documentation and examples.

- [ ] **Step 5: Run formatting, lint, tests, docs, and mechanical boundary checks**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --doc
git diff --check
rg -n "inspect_inputs|InputJournalEntry|InputsInRange|dispatch_inspection|input_log|WorkerAdmission|group_by_id" src benches README.md tests --glob '!public_core_api.rs'
```

Expected: formatting and strict Clippy pass; all unit, integration, trybuild, Criterion smoke, example, and doctests pass; the final `rg` returns no matches.

- [ ] **Step 6: Write the implementation report**

Create `docs/superpowers/reports/2026-08-26-snapshot-batched-apply-pipeline-report.md` containing:

- the base and final commit SHAs;
- the RED and GREEN evidence for Tasks 1 through 5;
- the exact verification commands and test counts;
- the four fresh Criterion intervals and adjacent residual interpretation;
- confirmation that inspection and worker admission symbols are absent; and
- the documented provisional memory-consistency limitation.

- [ ] **Step 7: Commit Task 6**

```bash
git add benches/apply_boundaries.rs benches/router.rs benches/helpers.rs benches/apply.rs README.md src/lib.rs docs/superpowers/reports/2026-08-26-snapshot-batched-apply-pipeline-report.md
git commit -m "docs: verify snapshot batched apply pipeline"
```

- [ ] **Step 8: Verify the exact committed HEAD**

Run:

```bash
git status --short
git log -1 --oneline
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --doc
```

Expected: clean working tree and every exact-HEAD gate passes.
