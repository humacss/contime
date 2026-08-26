# Hybrid History Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace per-input tree admission with an append-oriented ordered history plus a late-input B-tree while preserving ConTime's canonical ordering, replay, idempotency, and horizon behavior.

**Architecture:** Each snapshot history owns a `HistoryInputs` abstraction backed by an ordered `VecDeque` and a `BTreeMap` used only for late keys. Replay consumes a zero-copy merge iterator over both collections. The router separately owns timestamp-independent retained-ID lookup and a time-ordered retention index used only to forget IDs at the horizon.

**Tech Stack:** Rust 2021, standard-library `VecDeque`/`BTreeMap`/`HashSet`, AHash where already used, Crossbeam channels, Criterion 0.5, existing ConTime test fixtures.

**Spec:** `docs/superpowers/specs/2026-08-26-hybrid-history-storage-design.md`

## Global Constraints

- A retained input ID is a no-op regardless of the timestamp or payload on a repeated submission.
- Input identity is forgotten when the canonical input is pruned beyond the horizon.
- Canonical replay order remains `(complete_time, input_id)` and same-time inputs remain one batch.
- High-level apply, send, query, inspection, marker, checkpoint, and horizon APIs retain their behavior.
- Timeless Runtime, worker scheduling policy, checkpoint cadence, and concurrency semantics are outside this change.
- Preserve all pre-existing dirty worktree changes; never reset, overwrite, or stage unrelated hunks.
- Establish an isolated execution baseline from the current working state before Task 1. A clean worktree from `HEAD` alone is insufficient because the current source and tests contain uncommitted prerequisite work.

## File Structure

- Create `src/history/inputs.rs`: hybrid input storage, admission summaries, range-bound calculation, merged iteration, pruning, and focused unit tests.
- Modify `src/history/mod.rs`: register and re-export the advanced `HistoryInputs` type.
- Modify `src/lib.rs`: hidden re-export needed by Criterion without exposing backing fields.
- Modify `src/history/storage.rs`: store `HistoryInputs`, construct it, and prune it while retaining replay-anchor behavior.
- Modify `src/history/apply.rs`: delegate batch admission and metadata calculation to `HistoryInputs`.
- Modify `src/history/checkpoints.rs`: consume merged ranges and reuse one same-time bucket buffer.
- Modify `src/router.rs`: split global identity lookup from retention-time pruning.
- Modify `tests/edge.rs`: retained-ID behavior across different timestamps.
- Modify `tests/journal.rs`: horizon forgetting and reacceptance.
- Create `tests/hybrid_history.rs`: representation-independent randomized reference-model coverage.
- Modify `benches/apply.rs`: permanent callback, direct history, asynchronous routing, synchronous apply, late-rate, replay, and pruning benchmarks.
- Modify `README.md`: explain the hybrid storage boundary and record reproducible before/after benchmark results.
- Create `docs/superpowers/reports/2026-08-26-hybrid-history-storage-report.md`: implementation evidence and remaining-cost handoff.

---

### Task 1: Make the performance baseline permanent

**Files:**
- Modify: `benches/apply.rs`

**Interfaces:**
- Consumes: `SnapshotHistory<BenchSnapshot>::apply_input_batch`, `BenchContime::{apply,send}`, and the fixtures in `benches/helpers.rs`.
- Produces: Criterion groups named `snapshot_callback_same_snapshot`, `snapshot_history_same_snapshot`, `send_persistent_matrix`, and `sync_apply_persistent_matrix` with sizes `1`, `100`, and `1000`.

- [ ] **Step 1: Add the direct callback and direct history benchmark matrices**

Add helpers that build one ordered same-snapshot batch and apply it either directly to `BenchSnapshot` or through `SnapshotHistory`:

```rust
fn ordered_events(size: usize, snapshot_id: u128, first_id: u128) -> Vec<BenchEvent> {
    (0..size)
        .map(|offset| {
            let id = first_id + offset as u128;
            BenchEvent::Positive(snapshot_id, offset as i64, id, 1)
        })
        .collect()
}

fn benchmark_snapshot_callback_same_snapshot(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("snapshot_callback_same_snapshot");
    for size in [1_usize, 100, 1_000] {
        group.bench_function(BenchmarkId::from_parameter(size), |bencher| {
            bencher.iter_batched(
                || ordered_events(size, 0, 1),
                |events| {
                    let mut snapshot = BenchSnapshot::default();
                    let refs = events.iter().collect::<Vec<_>>();
                    contime::ApplyEvents::apply_events(
                        &mut snapshot,
                        contime::ApplyBatch {
                            snapshot_id: 0,
                            time: size as i64,
                            history_input_count: size as u64,
                            events: &refs,
                        },
                    );
                    black_box(snapshot)
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn benchmark_snapshot_history_same_snapshot(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("snapshot_history_same_snapshot");
    for size in [1_usize, 100, 1_000] {
        group.bench_function(BenchmarkId::from_parameter(size), |bencher| {
            bencher.iter_batched_ref(
                || {
                    (
                        SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10_000).0,
                        ordered_events(size, 0, 1),
                    )
                },
                |(history, events)| black_box(history.apply_input_batch(std::mem::take(events), &mut ())),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}
```

If the existing `ApplyBatch` fixture uses a different public field name, use the exact constructor already compiled in `benches/helpers.rs`; do not add a second benchmark-only event trait.

- [ ] **Step 2: Add persistent asynchronous and synchronous matrices**

Use one long-lived one-worker `BenchContime` per benchmark and monotonically increasing snapshot/event IDs so each iteration is unique:

```rust
fn next_event_batch(next_id: &mut u128, size: usize, one_snapshot: bool) -> Vec<BenchEvent> {
    let base = *next_id;
    *next_id = next_id.wrapping_add(size as u128 + 1);
    (0..size)
        .map(|offset| {
            let id = base + offset as u128;
            let snapshot_id = if one_snapshot { base } else { id };
            BenchEvent::Positive(snapshot_id, 0, id, 1)
        })
        .collect()
}
```

Register same-snapshot and separate-snapshot cases for `send` and `apply`, each at `1`, `100`, and `1000`. Convert the returned `Vec<BenchEvent>` with `batch.into_iter().map(Into::into)` at the API call. Call `inspect_inputs(..)` after asynchronous `send` during setup when a benchmark requires a completed worker frontier; do not include that wait in the timed closure.

- [ ] **Step 3: Compile the benchmark before running it**

Run:

```bash
cargo bench --bench apply --no-run
```

Expected: the `apply` benchmark target compiles with no warnings promoted to errors.

- [ ] **Step 4: Capture the focused before measurements**

Run:

```bash
cargo bench --bench apply -- snapshot_callback_same_snapshot --sample-size 20
cargo bench --bench apply -- snapshot_history_same_snapshot --sample-size 20
cargo bench --bench apply -- send_persistent_matrix --sample-size 20
cargo bench --bench apply -- sync_apply_persistent_matrix --sample-size 20
```

Expected: every `1`, `100`, and `1000` case reports a Criterion interval. Save the terminal output in the implementation report or task notes; these are the before values used by Task 6.

- [ ] **Step 5: Commit only the benchmark instrumentation**

```bash
git add benches/apply.rs
git commit -m "bench: isolate contime apply costs"
```

---

### Task 2: Build the hybrid `HistoryInputs` collection

**Files:**
- Create: `src/history/inputs.rs`
- Modify: `src/history/mod.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `crate::{ContimeKey, ContimeTime, Input}`.
- Produces:
  - `HistoryInputs<T, I>::new() -> Self`
  - `HistoryInputs<T, I>::insert_batch(Vec<I>) -> HistoryInsert<T>`
  - `HistoryInputs<T, I>::insert_batch_with(Vec<I>, FnMut(&I)) -> HistoryInsert<T>` for per-snapshot lane validation of newly inserted inputs only
  - `HistoryInputs<T, I>::iter() -> MergedInputs<'_, T, I>`
  - `HistoryInputs<T, I>::range((Bound<ContimeKey<T>>, Bound<ContimeKey<T>>)) -> MergedInputs<'_, T, I>`
  - `HistoryInputs<T, I>::prune_before(&ContimeKey<T>) -> PrunedInputs`
  - `HistoryInputs::{len,is_empty,latest_key,latest_key_before,storage_counts}`
  - `HistoryInsert<T>` carrying `inserted_count`, `bytes_delta`, `earliest_time`, `latest_key_before`, and `single_key`.
  - `PrunedInputs::{count,bytes}` reporting logically and physically dropped input payloads.

- [ ] **Step 1: Write failing append, late, merge, and pruning tests**

Create `src/history/inputs.rs` with a `#[cfg(test)]` module. Use a tiny `StoredTestInput` implementing `Input`. Add these explicit cases:

```rust
#[test]
fn ordered_batches_stay_in_the_append_deque() {
    let mut inputs = HistoryInputs::new();
    let inserted = inputs.insert_batch(vec![event(1, 10), event(2, 20), event(3, 30)]);
    assert_eq!(inserted.inserted_count(), 3);
    assert_eq!(inputs.storage_counts(), (3, 0));
    assert_eq!(keys(&inputs), vec![(10, 1), (20, 2), (30, 3)]);
}

#[test]
fn a_middle_input_uses_the_late_tree_and_merges_canonically() {
    let mut inputs = HistoryInputs::new();
    inputs.insert_batch(vec![event(1, 10), event(3, 30)]);
    inputs.insert_batch(vec![event(2, 20)]);
    assert_eq!(inputs.storage_counts(), (2, 1));
    assert_eq!(keys(&inputs), vec![(10, 1), (20, 2), (30, 3)]);
}

#[test]
fn same_time_inputs_are_merged_by_id() {
    let mut inputs = HistoryInputs::new();
    inputs.insert_batch(vec![event(30, 10), event(10, 10), event(20, 10)]);
    assert_eq!(keys(&inputs), vec![(10, 10), (10, 20), (10, 30)]);
}

#[test]
fn pruning_drops_ordered_and_late_payloads() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut inputs = HistoryInputs::new();
    inputs.insert_batch(vec![tracked_event(1, 10, &drops), tracked_event(3, 30, &drops)]);
    inputs.insert_batch(vec![tracked_event(2, 20, &drops)]);
    let pruned = inputs.prune_before(&ContimeKey { time: 25, id: u128::MIN });
    assert_eq!(pruned.count(), 2);
    assert_eq!(drops.load(Ordering::Relaxed), 2);
    assert_eq!(keys(&inputs), vec![(30, 3)]);
}
```

Also test inclusive/exclusive/unbounded ranges and `latest_key_before` across both stores.

- [ ] **Step 2: Run the new unit target and confirm RED**

Run:

```bash
cargo test history::inputs --lib
```

Expected: compilation fails because `HistoryInputs`, `HistoryInsert`, `MergedInputs`, and `PrunedInputs` are not implemented.

- [ ] **Step 3: Implement the collection and zero-copy merge iterator**

Implement the production structures:

```rust
#[derive(Debug, Clone)]
pub struct HistoryInputs<T, I>
where
    T: ContimeTime,
    I: Input<Time = T>,
{
    ordered: VecDeque<(ContimeKey<T>, I)>,
    late: BTreeMap<ContimeKey<T>, I>,
}

pub struct MergedInputs<'a, T, I>
where
    T: ContimeTime,
    I: Input<Time = T>,
{
    ordered: Peekable<std::collections::vec_deque::Iter<'a, (ContimeKey<T>, I)>>,
    late: Peekable<std::collections::btree_map::Range<'a, ContimeKey<T>, I>>,
}
```

`insert_batch` delegates to `insert_batch_with(inputs, |_| {})`. `insert_batch_with` must build each key once and invoke its callback exactly once for each genuinely inserted input before moving that input into storage. Its fast path verifies that the first key is greater than the existing tail and every adjacent key is strictly increasing, then extends `ordered`. Its fallback sorts by key, ignores an exact key already present in either store, inserts keys at or below the pre-insert tail into `late`, and appends the sorted suffix above that tail. Calculate `earliest_time` only from inputs actually inserted.

Implement range boundaries by finding the deque's start and end indexes with `partition_point` and taking `ordered.range(start_index..end_index)`. `MergedInputs::next` compares the two peeked keys and advances only the smaller source. Equal keys trigger a debug assertion and advance one canonical source rather than yielding twice in release builds.

Implement `prune_before` with `VecDeque::pop_front` and `BTreeMap::split_off`. Return both the removed logical input count and the sum of `Input::conservative_size` for removed inputs.

- [ ] **Step 4: Export the advanced collection without exposing its fields**

In `src/history/mod.rs`:

```rust
mod inputs;

#[doc(hidden)]
pub use inputs::{HistoryInputs, HistoryInsert};
```

In `src/lib.rs`, extend the existing history export:

```rust
#[doc(hidden)]
pub use history::{HistoryInputs, HistoryInsert};
pub use history::{ApplyInner, ApplyWrapper, SnapshotHistory};
```

Keep backing fields private. Expose `len`, `is_empty`, `storage_counts`, and `inserted_count` as ordinary methods so Criterion and advanced `SnapshotHistory` tests do not depend on representation.

- [ ] **Step 5: Run focused tests and formatting**

Run:

```bash
cargo fmt --check
cargo test history::inputs --lib
```

Expected: all new collection tests pass, including drop-count pruning and merged range bounds.

- [ ] **Step 6: Commit the isolated collection**

```bash
git add src/history/inputs.rs src/history/mod.rs src/lib.rs
git commit -m "feat: add hybrid history input store"
```

---

### Task 3: Integrate hybrid admission, replay, and pruning

**Files:**
- Modify: `src/history/storage.rs`
- Modify: `src/history/apply.rs`
- Modify: `src/history/checkpoints.rs`

**Interfaces:**
- Consumes: `HistoryInputs`, `HistoryInsert`, `MergedInputs`, and `PrunedInputs` from Task 2.
- Produces: `LocalSnapshotHistory::inputs: HistoryInputs<S::Time, S::Input>` with existing apply/query/advance behavior.

- [ ] **Step 1: Add RED integration tests around the storage boundary**

In the existing `src/history/storage.rs` test module, add:

```rust
#[test]
fn ordered_then_late_history_replays_across_both_stores() {
    let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
    let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1_000);
    apply_one(&mut history, TestEvent::Positive(1, 10, 10, 1));
    apply_one(&mut history, TestEvent::Positive(1, 30, 30, 3));
    apply_one(&mut history, TestEvent::Positive(1, 20, 20, 2));
    assert_eq!(history.inputs.storage_counts(), (2, 1));
    assert_eq!(history.snapshot_only_at(30).items, vec![1, 2, 3]);
}

#[test]
fn same_time_late_id_replays_the_complete_bucket_in_id_order() {
    let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
    let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1_000);
    apply_one(&mut history, TestEvent::Positive(1, 10, 20, 2));
    apply_one(&mut history, TestEvent::Negative(1, 10, 10, 1));
    assert_eq!(history.inputs.storage_counts(), (1, 1));
    assert_eq!(history.snapshot_only_at(10).items, vec![-1, 2]);
}
```

Add a horizon test that creates one ordered and one late input before the drop boundary, advances past them, and asserts `inputs.is_empty()` while the replay-anchor snapshot retains their state.

- [ ] **Step 2: Run focused tests and confirm RED**

Run:

```bash
cargo test ordered_then_late_history_replays_across_both_stores --lib
cargo test same_time_late_id_replays_the_complete_bucket_in_id_order --lib
```

Expected: compilation or assertions fail because `LocalSnapshotHistory` still uses the single B-tree.

- [ ] **Step 3: Switch construction, admission metadata, and pruning**

In `storage.rs`, replace both `BTreeMap::new()` constructors with `HistoryInputs::new()`. Replace `split_off` pruning with:

```rust
let pruned = self.inputs.prune_before(&drop_key);
bytes_delta -= pruned.bytes() as i64;
```

In `apply.rs`, remove direct `btree_map::Entry` usage. Delegate storage mutation to `self.inputs.insert_batch_with(inputs, callback)`. The callback performs snapshot-lane validation and materialization only for genuinely inserted event inputs. Borrow `snapshot_lane_index` separately from `inputs` before the call so the callback does not reborrow the whole history. Map the result into the existing `InsertedInputBatch` fields without changing checkpoint reconciliation behavior.

The admission result must distinguish inserted inputs from duplicates so duplicates cannot change `earliest_changed_time`, `changed_event_count`, lane materialization, memory deltas, or checkpoint selection.

- [ ] **Step 4: Convert checkpoint replay to the merged range**

Change `apply_input_buckets` to accept `&HistoryInputs<S::Time, S::Input>` rather than a `BTreeMap`. Use `inputs.range((start, end)).peekable()` exactly as the old replay loop used the B-tree range.

Move `let mut input_bucket = Vec::new();` before the outer bucket loop and call `input_bucket.clear()` before collecting each new complete-time bucket. This preserves references only for the callback duration and reuses allocation across replay buckets.

Replace the `next_back` call in `get_checkpoint_before_with_context` with `inputs.latest_key_before(&boundary)`. Replace materialization's raw range with the merged range. `latest_input_key` delegates to `inputs.latest_key()` and all count/cadence logic continues to use `inputs.len()`.

- [ ] **Step 5: Update existing representation-specific assertions**

In `storage.rs` tests, replace calls such as `history.inputs.keys()` and `history.inputs.values()` with the representation-neutral `history.inputs.iter()` or `history.inputs.range(..)`. Preserve every existing expected key, count, and snapshot value.

- [ ] **Step 6: Run history, checkpoint, marker, and horizon suites**

Run:

```bash
cargo fmt --check
cargo test history --lib
cargo test --test inputs
cargo test --test history_input_count
cargo test --test horizon_compaction
```

Expected: every command passes. Existing same-time marker grouping, checkpoint cadence, pending histories, replay anchors, and raw input counts remain unchanged.

- [ ] **Step 7: Commit the integrated history path**

```bash
git add src/history/storage.rs src/history/apply.rs src/history/checkpoints.rs
git commit -m "perf: append ordered snapshot history"
```

---

### Task 4: Separate retained identity from retention time

**Files:**
- Modify: `src/router.rs`
- Modify: `tests/edge.rs`
- Modify: `tests/journal.rs`

**Interfaces:**
- Consumes: existing `Router::route_inputs` and `Router::advance_to` flow.
- Produces: `CanonicalInputIndex<T> { retained_ids: HashSet<u128>, ids_by_retention_time: BTreeMap<T, Vec<u128>> }` where only `retained_ids` answers identity queries.

- [ ] **Step 1: Strengthen RED tests for timestamp-independent identity and horizon forgetting**

In `tests/edge.rs`, split the existing duplicate test into two apply calls so the second ID arrives after the first has completed:

```rust
#[test]
fn duplicate_event_id_at_a_different_time_is_a_noop() {
    let contime = TestSnapshotContime::new(1, 100_000);
    contime.apply([TestEvent::Positive(1, 1, 7, 10).into()]).unwrap();
    contime.apply([TestEvent::Positive(1, 5, 7, 20).into()]).unwrap();
    let snapshot = query_one(&contime, 6, 1);
    assert_eq!(snapshot.sum, 10);
    assert_eq!(contime.inspect_inputs(..).unwrap().len(), 1);
}
```

In `tests/journal.rs`, make the horizon test prove both halves of the contract:

```rust
#[test]
fn horizon_forgets_identity_and_allows_the_id_at_a_new_retained_time() {
    let contime = TestSnapshotContime::with_history_horizon(1, 100_000, 50);
    contime.apply([TestEvent::Positive(1, 10, 7, 10).into()]).unwrap();
    contime.apply([TestEvent::Positive(1, 11, 7, 99).into()]).unwrap();
    assert_eq!(contime.inspect_inputs(..).unwrap().len(), 1);

    contime.advance_to(70).unwrap();
    contime.apply([TestEvent::Positive(1, 30, 7, 20).into()]).unwrap();

    let retained = contime.inspect_inputs(20..).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!((retained[0].input.time(), retained[0].input.id()), (30, 7));
}
```

- [ ] **Step 2: Run the focused tests before changing the index**

Run:

```bash
cargo test --test edge duplicate_event_id_at_a_different_time_is_a_noop
cargo test --test journal horizon_forgets_identity_and_allows_the_id_at_a_new_retained_time
```

Expected: the first test documents current global deduplication; the second documents pruning/reacceptance. If both are already green, record that as characterization evidence and continue with the internal refactor.

- [ ] **Step 3: Replace the ID-to-time identity map**

Implement:

```rust
struct CanonicalInputIndex<T> {
    retained_ids: HashSet<u128>,
    ids_by_retention_time: BTreeMap<T, Vec<u128>>,
}

impl<T: ContimeTime> CanonicalInputIndex<T> {
    fn contains(&self, input_id: u128) -> bool {
        self.retained_ids.contains(&input_id)
    }

    fn insert(&mut self, input_id: u128, time: T) {
        assert!(self.retained_ids.insert(input_id), "canonical input ID was inserted twice");
        self.ids_by_retention_time.entry(time).or_default().push(input_id);
    }

    fn prune_before(&mut self, earliest_time: T) -> usize {
        let retained = self.ids_by_retention_time.split_off(&earliest_time);
        let removed = std::mem::replace(&mut self.ids_by_retention_time, retained);
        removed
            .into_values()
            .flatten()
            .filter(|input_id| self.retained_ids.remove(input_id))
            .count()
    }
}
```

Use one per-call `accepted_ids` set only for duplicates within the not-yet-committed batch. Preserve the rule that IDs enter the canonical index only after horizon and memory admission succeeds.

- [ ] **Step 4: Correct conservative index accounting**

Keep the deliberately conservative per-ID charge explicit:

```rust
const fn canonical_input_index_entry_size<T>() -> u64 {
    // One ID in retained_ids, one ID in the retention bucket, and a
    // conservatively repeated time charge even when IDs share a bucket.
    (size_of::<u128>() * 2 + size_of::<T>()) as u64
}
```

This preserves the existing budget ceiling while the actual representation becomes a `HashSet` plus time buckets.

- [ ] **Step 5: Run identity, journal, memory, and generic-time tests**

Run:

```bash
cargo fmt --check
cargo test --test edge
cargo test --test journal
cargo test --test memory
cargo test --test generic_time
```

Expected: all tests pass, including different-time no-op before pruning and same-ID acceptance after pruning.

- [ ] **Step 6: Commit the identity boundary**

```bash
git add src/router.rs tests/edge.rs tests/journal.rs
git commit -m "refactor: separate input identity from retention time"
```

---

### Task 5: Prove equivalence against a reference history

**Files:**
- Create: `tests/hybrid_history.rs`
- Modify: `src/history/inputs.rs`

**Interfaces:**
- Consumes: public hidden `HistoryInputs`, `HistoryInsert`, and existing `TestEvent` fixtures.
- Produces: deterministic randomized equivalence coverage and debug-only invariant validation.

- [ ] **Step 1: Add a deterministic reference-model test**

Use a local integer generator rather than adding a random-number dependency:

```rust
fn next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *state
}
```

For each of 32 fixed seeds, execute 1,000 operations. Generate approximately 80% ordered appends, 15% late insertions within the retained range, and 5% horizon advances. Maintain:

```rust
let mut reference = BTreeMap::<(i64, u128), TestEvent>::new();
let mut retained_ids = HashSet::<u128>::new();
```

Before every insertion, apply the same ID-first no-op rule to the reference and filter retained duplicate IDs from the batch passed to `HistoryInputs`; global router admission owns that filtering in production. On pruning, remove all reference entries before `(drop_time, u128::MIN)` and remove their IDs from `retained_ids`. After every operation, compare the complete merged `(time, id)` sequence, `len`, `is_empty`, `latest_key`, and removed byte/count totals.

- [ ] **Step 2: Run the reference test and confirm any mismatch is visible**

Run:

```bash
cargo test --test hybrid_history -- --nocapture
```

Expected before final invariant hardening: either PASS or a deterministic assertion that prints the seed and operation index. Temporarily reverse the merge comparison in the test, confirm it fails, then restore it before continuing.

- [ ] **Step 3: Add mutation-boundary invariant checks**

Implement a debug-only method in `HistoryInputs`:

```rust
#[cfg(debug_assertions)]
fn assert_invariants(&self) {
    assert!(self.ordered.iter().map(|(key, _)| key).is_sorted());
    assert!(self.ordered.iter().map(|(key, _)| key).zip(self.ordered.iter().skip(1).map(|(key, _)| key)).all(|(a, b)| a < b));
    if let Some((ordered_tail, _)) = self.ordered.back() {
        assert!(self.late.keys().all(|key| key < ordered_tail));
    }
    assert_eq!(self.len(), self.iter().count());
}
```

Call it after successful batch admission and pruning. In release builds, provide an empty inline method so call sites do not need conditional compilation.

- [ ] **Step 4: Run equivalence and all integration tests**

Run:

```bash
cargo fmt --check
cargo test --test hybrid_history
cargo test --tests
```

Expected: the randomized reference model and every existing integration test pass.

- [ ] **Step 5: Commit equivalence coverage**

```bash
git add src/history/inputs.rs tests/hybrid_history.rs
git commit -m "test: verify hybrid history equivalence"
```

---

### Task 6: Measure ordered, late, replay, and pruning costs

**Files:**
- Modify: `benches/apply.rs`

**Interfaces:**
- Consumes: benchmark groups from Task 1 and `HistoryInputs::storage_counts` from Task 2.
- Produces: Criterion groups `history_late_rate`, `history_merged_replay`, and `history_horizon_prune`.

- [ ] **Step 1: Add late-rate benchmark fixtures**

Create batches of 1,000 inputs with deterministic late rates of `0`, `1`, `10`, and `50` percent. Establish an ordered tail first; for each late percentage, move exactly that percentage of keys into earlier gaps while keeping IDs unique. Assert the expected `(ordered, late)` counts in untimed setup before passing the history to `black_box`.

Register:

```rust
for late_percent in [0_u32, 1, 10, 50] {
    group.bench_function(BenchmarkId::new("1000_inputs", late_percent), |bencher| {
        bencher.iter_batched_ref(
            || history_and_batch_for_late_rate(late_percent),
            |(history, batch)| {
                history.apply_input_batch(std::mem::take(batch), &mut ());
                black_box(history.inputs.storage_counts())
            },
            BatchSize::SmallInput,
        );
    });
}
```

- [ ] **Step 2: Add merged replay and horizon-pruning groups**

For merged replay, prebuild histories with 1,000 inputs and late-tree densities of `0`, `10`, and `50` percent, then time `snapshot_at(1_000)`.

For pruning, prebuild ordered-only, late-only, and mixed histories whose first 500 inputs precede the boundary. Time one `advance(1_500)` call and assert in setup that the configured horizon produces a drop boundary of `500`.

- [ ] **Step 3: Compile and run the focused post-change matrix**

Run:

```bash
cargo bench --bench apply --no-run
cargo bench --bench apply -- snapshot_history_same_snapshot --sample-size 20
cargo bench --bench apply -- history_late_rate --sample-size 20
cargo bench --bench apply -- history_merged_replay --sample-size 20
cargo bench --bench apply -- history_horizon_prune --sample-size 20
cargo bench --bench apply -- send_persistent_matrix --sample-size 20
cargo bench --bench apply -- sync_apply_persistent_matrix --sample-size 20
```

Expected: all cases complete and Criterion reports intervals. Compare `snapshot_history_same_snapshot` and the persistent matrices directly to Task 1's saved before output. Do not attribute remaining synchronous latency to history when the direct history result is already fast.

- [ ] **Step 4: Commit the expanded benchmark matrix**

```bash
git add benches/apply.rs
git commit -m "bench: measure hybrid history workloads"
```

---

### Task 7: Document results and verify the crate

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: exact Criterion intervals from Tasks 1 and 6.
- Produces: reproducible performance documentation and a fully verified implementation branch.

- [ ] **Step 1: Update README architecture documentation**

In `How it Works`, add a `History storage` subsection explaining:

```text
Each snapshot history stores canonically ordered arrivals in an array-backed
append deque. Inputs that arrive before the append tail are kept in a separate
B-tree. Replay merges the two already-ordered sources by (time, input ID), so
late events preserve deterministic history without making normal ordered
admission pay for a tree insertion. The retained-ID set is independent of
timestamps and is pruned with the history horizon.
```

State that `LocalSnapshotHistory::inputs` is now a representation-neutral
`HistoryInputs` value and direct users should use its iteration and measurement
methods rather than relying on `BTreeMap` methods.

- [ ] **Step 2: Record exact benchmark evidence**

Replace the README statement that benchmarks are not current with:

- CPU model, OS, Rust version, profile, Criterion sample size, and measurement date.
- One before/after table for direct callback, direct history at `1/100/1000`, persistent `send`, and persistent synchronous `apply`.
- One workload table for late rates, merged replay densities, and ordered/late/mixed pruning.
- The exact commands from Task 6.
- A boundary note that callback cost, history admission/replay, routing, worker wake-up, and outer Runtime orchestration are distinct measurements.

Copy Criterion's `time: [low estimate high]` interval for each row exactly from the saved output. Do not round different units into one unit and do not claim a speedup when intervals overlap.

- [ ] **Step 3: Run formatting, strict linting, tests, docs, and benchmark compilation**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --doc
cargo bench --bench apply --no-run
```

Expected: every command exits successfully. Report the exact unit, integration, and doctest counts from the command output.

- [ ] **Step 4: Inspect the final diff for scope and accidental staging**

Run:

```bash
git diff --check
git status --short
git diff -- README.md
```

Expected: no whitespace errors; README contains measured values and commands; unrelated pre-existing worktree changes remain distinguishable and unstaged unless they were explicitly incorporated into the execution baseline.

- [ ] **Step 5: Commit documentation only**

```bash
git add README.md
git commit -m "docs: record hybrid history performance"
```

- [ ] **Step 6: Write the implementation report**

Create `docs/superpowers/reports/2026-08-26-hybrid-history-storage-report.md` containing:

- commit SHAs for Tasks 1 through 6 and the Task 7 README commit;
- RED and GREEN evidence for each task;
- before/after Criterion intervals;
- correctness and lint command results;
- remaining routing and worker wake-up costs;
- confirmation that Timeless Runtime was not modified.

Run `git diff --check` on the report, then commit it separately:

```bash
git add docs/superpowers/reports/2026-08-26-hybrid-history-storage-report.md
git commit -m "docs: report hybrid history implementation"
```
