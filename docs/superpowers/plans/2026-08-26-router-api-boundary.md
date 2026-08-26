# Router and API Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make ConTime routing a non-blocking, allocation-conscious partition-and-dispatch operation while moving synchronous response collection and public rejection aggregation into the `Contime` API.

**Architecture:** Every synchronous API call creates one request-scoped channel. The router partitions by snapshot ID, sends at most one message per affected worker, returns the dispatch count, and never waits. Workers respond directly to the API channel once per worker batch; `send` supplies no response channel. Normal admission failures are deduplicated `EventRejection` values, while channel failures remain non-generic infrastructure errors.

**Tech Stack:** Rust 2021, `crossbeam-channel`, `ahash`, generated lane macros, Criterion, standard Rust test harness.

**Spec:** `docs/superpowers/specs/2026-08-26-router-api-boundary-design.md`

## Global Constraints

- The router owns only worker senders, worker count/hash configuration, route extraction, request-level worker batches, dispatch, and affected-worker counting.
- The router never receives from a worker channel and never waits.
- `send` creates no response channel and workers send no response for it.
- `apply`, `query_at`, `inspect_inputs`, and `advance_to` use one distinct response channel per API call and continue to accept `&self`, allowing concurrent calls.
- Worker application remains infallible; horizon and memory decisions are event rejection values.
- A rejection exposes exactly `event_id` and `reason`.
- Identical `(event_id, reason)` pairs are deduplicated; different reasons for one event remain distinct.
- Snapshot route extraction allocates no `Vec<u128>` per input.
- Do not optimize worker journal, worker regrouping, history bulk admission, checkpoint replay, or snapshot hash caching beyond changes required to establish this boundary.
- Timeless Runtime is out of scope.

---

### Task 1: Replace apply outcomes with direct event rejections

**Files:**
- Modify: `src/api.rs:1-55`
- Modify: `src/lib.rs:60-68`
- Modify: `src/router.rs:1-310`
- Modify: `tests/inputs.rs`
- Modify: `tests/generic_time.rs`
- Test: `src/api.rs` unit tests

**Interfaces:**
- Consumes: existing `Input::id()` values and current public error types.
- Produces: `EventRejection`, `EventRejectionReason`, `merge_event_rejections`, and direct rejection-vector apply results used by later tasks.

- [ ] **Step 1: Write failing API model tests**

Add an `api::tests` module that specifies empty success, duplicate rejection coalescing, and preservation of distinct reasons:

```rust
#[test]
fn rejection_merge_deduplicates_only_identical_event_and_reason_pairs() {
    let mut merged = vec![
        EventRejection::new(7, EventRejectionReason::MemoryFull),
        EventRejection::new(7, EventRejectionReason::BeforeHistoryHorizon),
    ];
    merge_event_rejections(
        &mut merged,
        vec![
            EventRejection::new(7, EventRejectionReason::MemoryFull),
            EventRejection::new(9, EventRejectionReason::MemoryFull),
        ],
    );

    assert_eq!(
        merged,
        vec![
            EventRejection::new(7, EventRejectionReason::BeforeHistoryHorizon),
            EventRejection::new(7, EventRejectionReason::MemoryFull),
            EventRejection::new(9, EventRejectionReason::MemoryFull),
        ]
    );
}

#[test]
fn empty_rejection_vector_is_the_success_value() {
    let mut merged = Vec::new();
    merge_event_rejections(&mut merged, Vec::new());
    assert!(merged.is_empty());
    assert_eq!(merged.capacity(), 0);
}
```

- [ ] **Step 2: Run the focused tests and capture RED**

Run:

```bash
cargo test api::tests::rejection_merge -- --nocapture
```

Expected: compile failure because `EventRejection`, `EventRejectionReason`, and `merge_event_rejections` do not exist.

- [ ] **Step 3: Implement the direct rejection model**

Replace `ApplyOutcome`, `InputRejection`, and `InputRejectionReason` with:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventRejectionReason {
    BeforeHistoryHorizon,
    MemoryFull,
}

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

pub(crate) fn merge_event_rejections(target: &mut Vec<EventRejection>, incoming: Vec<EventRejection>) {
    if incoming.is_empty() {
        return;
    }
    target.extend(incoming);
    target.sort_unstable();
    target.dedup();
}
```

Keep the existing generic `ContimeError<T>` and its memory/horizon variants in this task so the still-router-owned admission path remains compilable. Task 4 makes the error non-generic when those decisions move to workers. Export `EventRejection`, `EventRejectionReason`, `Contime`, and the transitional `ContimeError` from `lib.rs`; remove the old outcome and rejection types completely.

- [ ] **Step 4: Update outcome assertions without changing dispatch yet**

Change existing horizon tests to assert direct vectors. For example:

```rust
assert_eq!(
    rejections,
    vec![EventRejection::new(200, EventRejectionReason::BeforeHistoryHorizon)]
);
```

Change `Router::route_inputs` to return `(RoutedWorkerInputs<IL>, Vec<EventRejection>)`, `Router::apply` to return `Vec<EventRejection>`, and `Router::send` to discard the route rejection vector and return `()`. Keep the existing router memory-full error behavior until Task 4. Do not update memory-full tests yet.

Change the transitional public signatures to:

```rust
pub fn apply<I>(&self, inputs: I) -> Result<Vec<EventRejection>, ContimeError<SL::Time>>;
pub fn send<I>(&self, inputs: I) -> Result<(), ContimeError<SL::Time>>;
```

Task 4 removes the error type's time parameter after admission leaves the router.

- [ ] **Step 5: Run the model tests and compile all test targets**

Run:

```bash
cargo test api::tests -- --nocapture
cargo test --tests --no-run
```

Expected: API model tests pass and all test targets compile. Existing memory-full tests continue to pass against the transitional generic infrastructure error; Task 4 changes their asserted result.

- [ ] **Step 6: Commit**

```bash
git add src/api.rs src/lib.rs src/router.rs tests/inputs.rs tests/generic_time.rs
git commit -m "refactor: simplify apply rejection results"
```

---

### Task 2: Replace allocated snapshot-ID routes with a visitor

**Files:**
- Modify: `src/traits.rs:105-150`
- Modify: `contime_macros/src/lib.rs:270-305,530-545`
- Modify: `src/router.rs:230-290`
- Modify: `tests/derive_macros.rs`
- Modify: `tests/fragments.rs`
- Modify: `tests/history_input_count.rs`
- Modify: `tests/inputs.rs`

**Interfaces:**
- Consumes: generated event route tables and manual `InputRoute` implementations.
- Produces: `InputRoute::visit_snapshot_ids` and `InputLanes::visit_snapshot_ids`, consumed by the pure router in Task 4.

- [ ] **Step 1: Write failing single-target, multi-target, marker, and empty-route tests**

Replace tests that collect `snapshot_ids()` with a helper:

```rust
fn visited_snapshot_ids<SL, IL>(input: &IL) -> Vec<u128>
where
    SL: Snapshot<Input = IL>,
    IL: InputLanes<SL>,
{
    let mut ids = Vec::new();
    input.visit_snapshot_ids(&mut |id| ids.push(id));
    ids
}
```

Add assertions covering one generated target, several generated targets in declaration order, a marker's dynamic targets, and an empty marker route.

- [ ] **Step 2: Run the focused macro and route tests and capture RED**

Run:

```bash
cargo test derived_event_route_initializes_only_snapshot_identity -- --nocapture
cargo test snapshot_fragment_exposes_each_concrete_event_route -- --nocapture
cargo test unrouted_inputs_do_not_consume_the_memory_budget_or_enter_history -- --nocapture
```

Expected: compile failure because `visit_snapshot_ids` is missing.

- [ ] **Step 3: Change both routing traits**

Use the exact generic visitor signature:

```rust
pub trait InputRoute {
    fn visit_snapshot_ids<F>(&self, visit: &mut F)
    where
        F: FnMut(u128);
}

pub trait InputLanes<SL: Snapshot<Input = Self>>: Input<Time = SL::Time> + Clone {
    fn visit_snapshot_ids<F>(&self, visit: &mut F)
    where
        F: FnMut(u128);

    fn is_event(&self) -> bool;
    fn apply_events(snapshot: &mut SL, batch: InputBatch<'_, Self>, history_input_count: u64);
}
```

The blanket event implementation calls `visit(self.snapshot_id())` directly.

- [ ] **Step 4: Generate visitor calls instead of vectors**

For generated event routes, emit:

```rust
Self::Variant(event) => {
    visit(<Event as SnapshotEvent<TargetA>>::snapshot_id(event));
    visit(<Event as SnapshotEvent<TargetB>>::snapshot_id(event));
}
```

For markers, delegate to `InputRoute::visit_snapshot_ids(marker, visit)`. Remove every generated `vec![...]` route result.

- [ ] **Step 5: Adapt the current router with one request-scoped scratch buffer**

Before the later router rewrite, make the current route loop compile by declaring one `Vec<(u128, usize)>` outside the input loop, clearing it per input, and filling it through the visitor. Do not allocate a routed-worker-index vector per input.

- [ ] **Step 6: Run route, macro, UI, and integration tests**

Run:

```bash
cargo test derived_event_route_initializes_only_snapshot_identity -- --nocapture
cargo test fragments -- --nocapture
cargo test inputs -- --nocapture
cargo test history_input_count -- --nocapture
cargo test ui -- --nocapture
```

Expected: all focused targets pass.

- [ ] **Step 7: Commit**

```bash
git add src/traits.rs contime_macros/src/lib.rs src/router.rs tests/derive_macros.rs tests/fragments.rs tests/history_input_count.rs tests/inputs.rs
git commit -m "refactor: visit snapshot routes without allocation"
```

---

### Task 3: Add request-scoped worker completion messages

**Files:**
- Modify: `src/worker.rs:15-235`
- Modify: `src/router.rs:180-230`
- Modify: `tests/apply_context.rs`
- Test: `src/worker.rs` unit tests

**Interfaces:**
- Consumes: `EventRejection` from Task 1.
- Produces: `Completion<T>` and exactly-one-response worker behavior consumed by Task 4.

- [ ] **Step 1: Write failing completion-behavior tests**

Keep and strengthen `send_event_returns_after_enqueue_without_waiting_for_apply` in `tests/apply_context.rs`. Its existing `BlockingApplyTrace` must send `entered_tx`, block on `release_rx`, and record the applied snapshot only after release. Call `send`, require `entered_rx.recv_timeout(Duration::from_secs(1))`, assert the applied list is empty, release the worker, and query the resulting snapshot. This proves the API returned while application was still blocked without using a sleep.

Add a `BatchSizeTrace` apply wrapper whose only observation is `tx.send(batch.inputs.len())` before delegating to `apply_inner.apply_input_batch(batch)`. Submit exactly 1,000 distinct events with the same `entity_id` in one `apply` call to a one-worker instance, then assert:

```rust
assert!(contime.apply(inputs).unwrap().is_empty());
assert_eq!(batch_size_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 1_000);
assert!(batch_size_rx.try_recv().is_err());
```

In `src/worker.rs`, add a unit test around the completion-send helper. Give it a bounded response channel with capacity two, complete one batch, assert the first receive is the supplied rejection vector, and assert `try_recv()` returns `TryRecvError::Empty`. This directly specifies exactly one response per worker batch.

- [ ] **Step 2: Run focused tests and capture RED**

Run:

```bash
cargo test --test apply_context send_event_returns_after_enqueue_without_waiting_for_apply -- --nocapture
cargo test --test apply_context one_worker_applies_one_complete_thousand_input_batch -- --nocapture
cargo test worker::tests::responding_completion_sends_exactly_one_batch_result -- --nocapture
```

Expected: failure because input messages always contain a reply sender and the router owns the wait.

- [ ] **Step 3: Add completion mode to input messages**

Add:

```rust
pub(crate) enum Completion<T> {
    None,
    Respond(Sender<T>),
}

WorkerInbound::Inputs {
    inputs: Vec<WorkerInput<IL>>,
    completion: Completion<Vec<EventRejection>>,
}
```

`send` supplies `Completion::None`; the temporary synchronous path supplies `Respond`.

- [ ] **Step 4: Process one API worker batch as one response unit**

Remove `collect_replay_batch`. It cannot merge independent request messages once responses contain request-specific outcomes. Process each `WorkerInbound::Inputs` message independently and respond exactly once after its complete batch:

```rust
let rejections = process_worker_batch(..., inputs);
if let Completion::Respond(response) = completion {
    let _ = response.send(rejections);
}
```

For this task `process_worker_batch` returns an empty vector unconditionally; Task 4 moves admission decisions into it.

- [ ] **Step 5: Make the transitional router compile without changing public aggregation**

Adapt the existing synchronous router path to receive the one worker response and temporarily merge/ignore it only long enough to keep the crate compiling. Mark the transitional helper crate-private and remove it in Task 4.

- [ ] **Step 6: Run completion and apply-context suites**

Run:

```bash
cargo test --test apply_context -- --nocapture
cargo test worker::tests::responding_completion_sends_exactly_one_batch_result -- --nocapture
```

Expected: `send` does not wait, one worker produces one response for one batch, and existing apply-context behavior passes.

- [ ] **Step 7: Commit**

```bash
git add src/worker.rs src/router.rs tests/apply_context.rs
git commit -m "refactor: add request scoped worker completion"
```

---

### Task 4: Make input routing pure and aggregate apply responses in the API

**Files:**
- Create: `src/worker/admission.rs`
- Modify: `src/worker.rs`
- Modify: `src/router.rs`
- Modify: `src/api.rs`
- Modify: `src/lib.rs`
- Create: `tests/router_api_boundary.rs`
- Modify: `tests/edge.rs`
- Modify: `tests/journal.rs`
- Modify: `tests/memory.rs`
- Modify: `tests/inputs.rs`
- Modify: `tests/generic_time.rs`

**Interfaces:**
- Consumes: visitor routing from Task 2 and `Completion<Vec<EventRejection>>` from Task 3.
- Produces: pure `Router::dispatch_inputs`, worker-local admission, `send -> Result<(), ContimeError>`, and `apply -> Result<Vec<EventRejection>, ContimeError>`.

- [ ] **Step 1: Strengthen RED tests for public behavior and router non-waiting**

Add these concrete cases:

- `router::tests::dispatch_inputs_reports_only_affected_workers`: construct an eight-worker router with `RandomState::with_seeds(1, 2, 3, 4)`. Search monotonically increasing snapshot IDs until two IDs map to two distinct workers, dispatch one event to each, and assert the returned count is `2`. Verify only those two worker receivers contain one input message and the remaining six are empty. This is the authoritative subset-wait count used by `Contime::apply`.
- `concurrent_apply_calls_receive_only_their_own_rejections`: create `Arc<Contime>`, advance beyond the retained horizon, use a barrier to launch two `apply` calls concurrently with stale event IDs `101` and `202`, join both threads, and assert each result contains only its own `BeforeHistoryHorizon` rejection.
- `identical_rejections_from_multiple_workers_are_returned_once`: use a `WorkerIdTraceSender` apply-context factory to discover two snapshot IDs routed to distinct workers, advance the horizon, then apply one stale `SuppressInput` whose dynamic route contains both IDs. Assert the public result is exactly one rejection for the marker event ID.

The Task 1 aggregation unit test remains the authoritative test that two different reasons for one event are preserved. Keep the deterministic hasher constructor private under `#[cfg(test)]`; production hashing remains unchanged.

Update horizon tests to compare only `EventRejection::new(id, BeforeHistoryHorizon)`. Update memory tests to expect `MemoryFull` rejections instead of `ContimeError::MemoryFull`.

- [ ] **Step 2: Run focused public tests and capture RED**

Run:

```bash
cargo test --test router_api_boundary -- --nocapture
cargo test test_event_before_history_horizon_is_reported -- --nocapture
cargo test test_memory_full -- --nocapture
```

Expected: failures because admission still occurs in the router and the API does not own response aggregation.

- [ ] **Step 3: Move retained identity and horizon admission into worker state**

Create `WorkerAdmission<T>` in `src/worker/admission.rs`:

```rust
pub(crate) struct WorkerAdmission<T> {
    retained_ids: HashSet<u128>,
    ids_by_retention_time: BTreeMap<T, Vec<u128>>,
    current_time: T,
    horizon_delta: T,
}
```

It must:

- silently discard IDs retained by this worker from earlier requests;
- treat repeated routes for one new event inside the current worker batch as one identity while retaining every snapshot route;
- reject new events before `current_time.saturating_sub(horizon_delta)`;
- forget IDs when `advance_to` prunes their retention bucket;
- return one rejection per event/reason from the worker batch.

- [ ] **Step 4: Move memory admission into worker processing**

Pass both shared `memory_budget` and `memory_usage` atomics into each worker. For each unique newly admitted event, calculate the same conservative event/journal/history estimate previously calculated by the router. Attempt an atomic reservation before mutation:

```rust
fn try_reserve(memory_usage: &AtomicU64, budget: u64, bytes: u64) -> bool {
    memory_usage
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
            used.checked_add(bytes).filter(|next| *next < budget)
        })
        .is_ok()
}
```

Reject the event with `MemoryFull` if reservation fails. After journal/history mutation, reconcile `actual_delta - reserved_delta` exactly once so accounting is not doubled. A duplicate or stale event reserves nothing.

- [ ] **Step 5: Replace router admission with pure dispatch**

Delete from `Router`:

- `memory_budget` and `memory_usage` runtime admission fields;
- `canonical_inputs`;
- `route_inputs`, `apply`, and response receiving;
- outcome, rejection, horizon, identity, journal-size, and memory checks.

Leave current-time ownership and the old synchronous query/inspection/advance methods in place only until Task 5, where they move together to the API.

Implement:

```rust
pub(crate) fn dispatch_inputs<I>(
    &self,
    inputs: I,
    response: Option<&Sender<Vec<EventRejection>>>,
) -> Result<usize, RouterError>
where
    I: IntoIterator<Item = IL>;
```

It prepares request-level worker buckets, sends one message per non-empty bucket, clones `response` once per affected worker, and returns the number dispatched. `RouterError` becomes non-generic and contains only `WorkerUnavailable`.

- [ ] **Step 6: Move `send` and `apply` behavior into `Contime`**

At this boundary, replace the transitional generic error with the final infrastructure-only type:

```rust
#[derive(Debug)]
pub enum ContimeError {
    WorkerUnavailable,
    ResponseDisconnected,
}
```

Implement:

```rust
pub fn send<I>(&self, inputs: I) -> Result<(), ContimeError>
where
    I: IntoIterator<Item = IL>,
{
    self.router.dispatch_inputs(inputs, None)?;
    Ok(())
}

pub fn apply<I>(&self, inputs: I) -> Result<Vec<EventRejection>, ContimeError>
where
    I: IntoIterator<Item = IL>,
{
    let (response_tx, response_rx) = crossbeam_channel::unbounded();
    let expected = self.router.dispatch_inputs(inputs, Some(&response_tx))?;
    drop(response_tx);

    let mut rejections = Vec::new();
    for _ in 0..expected {
        let worker_rejections = response_rx.recv().map_err(|_| ContimeError::ResponseDisconnected)?;
        merge_event_rejections(&mut rejections, worker_rejections);
    }
    Ok(rejections)
}
```

Do not expose dispatch count or channels publicly.

- [ ] **Step 7: Remove transitional outcome and router-waiting code**

Delete every reference to `ApplyOutcome`, `InputRejection`, `InputRejectionReason`, `accepted_input_ids`, router-created apply reply channels, and canonical router identity indexes. Verify with:

```bash
rg "ApplyOutcome|InputRejection|accepted_input_ids|canonical_inputs|fn apply" src/router.rs src/api.rs src/lib.rs
```

Expected: no legacy outcome/identity symbols; `fn apply` appears only in the API and history/application traits, never as a router wait method.

- [ ] **Step 8: Run input, memory, journal, edge, generic-time, and concurrency tests**

Run:

```bash
cargo test --test router_api_boundary -- --nocapture
cargo test --test inputs -- --nocapture
cargo test --test memory -- --nocapture
cargo test --test journal -- --nocapture
cargo test --test edge -- --nocapture
cargo test --test generic_time -- --nocapture
```

Expected: all pass, including partial rejection, duplicate no-op, horizon forgetting, and concurrent request isolation.

- [ ] **Step 9: Commit**

```bash
git add src/worker/admission.rs src/worker.rs src/router.rs src/api.rs src/lib.rs tests/router_api_boundary.rs tests/edge.rs tests/journal.rs tests/memory.rs tests/inputs.rs tests/generic_time.rs
git commit -m "refactor: make input routing non blocking"
```

---

### Task 5: Move query, inspection, and advancement aggregation into the API

**Files:**
- Modify: `src/router.rs`
- Modify: `src/api.rs`
- Modify: `src/worker.rs`
- Modify: `tests/query.rs`
- Modify: `tests/journal.rs`
- Modify: `tests/memory.rs`
- Modify: `tests/router_api_boundary.rs`

**Interfaces:**
- Consumes: request-scoped response-channel pattern from Task 4.
- Produces: dispatch-only query, inspection, and advancement router methods; API-owned current time and aggregation.

- [ ] **Step 1: Write failing tests proving all router dispatches are non-blocking**

Add blocked-worker tests for query, inspection, and advancement. Each test must observe that dispatch returns before releasing the worker, then observe that the corresponding public API call remains blocked until the worker responds.

- [ ] **Step 2: Run focused boundary tests and capture RED**

Run:

```bash
cargo test --test router_api_boundary query_dispatch -- --nocapture
cargo test --test router_api_boundary inspection_dispatch -- --nocapture
cargo test --test router_api_boundary advance_dispatch -- --nocapture
```

Expected: tests fail because router methods still receive and merge responses.

- [ ] **Step 3: Add dispatch-only router methods**

Implement non-waiting methods with operation-specific response senders:

```rust
dispatch_query(time, positioned_snapshot_ids, &response_tx) -> Result<usize, RouterError>
dispatch_inspection(start, end, &response_tx) -> Result<usize, RouterError>
dispatch_advance(time, &response_tx) -> Result<usize, RouterError>
```

Each method sends at most one message per affected worker and returns the number sent. None calls `recv`.

- [ ] **Step 4: Move query aggregation into `Contime::query_at`**

Create the request channel in the API, dispatch positioned snapshot requests, drop the original sender, receive the expected worker vectors, and restore caller order in a result vector sized to the original request.

- [ ] **Step 5: Move inspection merge into `Contime::inspect_inputs`**

Move `merge_snapshot_ids` and canonical `(time, input ID)` merge logic from `router.rs` to `api.rs`. Receive one ordered vector per worker and return one globally ordered input entry with merged sorted snapshot IDs.

- [ ] **Step 6: Move time ownership and advancement waits into `Contime`**

Add `current_time: RwLock<SL::Time>` and `lower_time_horizon_delta: SL::Time` to `Contime`. `advance_to` updates API time, dispatches to every worker, and receives exactly one unit response per worker. `current_time` reads API state. The router broadcasts only.

- [ ] **Step 7: Remove every router receive and merge path**

Run:

```bash
rg "\.recv\(|merge_snapshot_ids|current_time|canonical_inputs|inspect_inputs|query_at|advance_to" src/router.rs
```

Expected: no `.recv`, result merge, current-time ownership, or public synchronous operation implementation remains in the router. Dispatch method names may remain; old synchronous names may not.

- [ ] **Step 8: Run focused and broad behavioral tests**

Run:

```bash
cargo test --test router_api_boundary -- --nocapture
cargo test --test query -- --nocapture
cargo test --test journal -- --nocapture
cargo test --test memory -- --nocapture
cargo test --test apply_context -- --nocapture
```

Expected: all pass.

- [ ] **Step 9: Commit**

```bash
git add src/router.rs src/api.rs src/worker.rs tests/query.rs tests/journal.rs tests/memory.rs tests/router_api_boundary.rs
git commit -m "refactor: aggregate worker responses in api"
```

---

### Task 6: Isolate router allocation and latency benchmarks

**Files:**
- Create: `src/router/partition.rs`
- Modify: `src/router.rs`
- Modify: `src/lib.rs`
- Create: `benches/router.rs`
- Create: `tests/router_allocations.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: allocation-free route visitor and pure dispatch preparation.
- Produces: one shared `RoutePartitioner` used by production routing, Criterion, and allocation regression tests.

- [ ] **Step 1: Extract the pure partitioner without changing behavior**

Move request-level bucketing into `src/router/partition.rs`:

```rust
pub(crate) struct RoutePartitioner {
    worker_count: usize,
    hasher: RandomState,
}

impl RoutePartitioner {
    pub(crate) fn partition<SL, IL, I>(&self, inputs: I) -> Vec<Vec<WorkerInput<IL>>>
    where
        SL: SnapshotLanes<Input = IL>,
        IL: InputLanes<SL>,
        I: IntoIterator<Item = IL>;
}
```

Production `Router::dispatch_inputs` must call this exact function. Add a `#[doc(hidden)]` benchmark adapter that reports bucket counts without exposing worker handles or channels.

- [ ] **Step 2: Write an allocation regression test and capture RED**

Create an isolated integration-test binary with a counting `GlobalAlloc` wrapper around `System`. Build 1,000 inputs before starting the counter, run the pure one-worker partition, and assert that allocations are bounded request-level allocations rather than input-count allocations:

```rust
assert!(allocations <= 8, "router allocated {allocations} times for one 1,000-event worker batch");
assert_eq!(routed_events, 1_000);
```

Run:

```bash
cargo test --test router_allocations -- --nocapture --test-threads=1
```

Expected before final capacity tuning: FAIL because allocations exceed the request-level bound.

- [ ] **Step 3: Reserve request-level buckets from iterator size hints**

Use the iterator lower size hint to reserve the common single-worker batch exactly and distribute a conservative initial capacity for multiple workers. Keep one request scratch route vector. Do not introduce per-input route or worker-index vectors.

- [ ] **Step 4: Add router-only Criterion cases**

Register a new `router` benchmark target with:

```text
router_partition/single_target_one_worker/{1,100,1000}
router_partition/single_target_eight_workers/{1,100,1000}
router_partition/multi_target_eight_workers/{1,100,1000}
router_enqueue/one_worker/{1,100,1000}
api_completion/empty_rejections/{1,2,8 workers}
```

Construct inputs and channels outside the timed partition region. Do not include horizon advancement, history mutation, or worker replay in router-only cases.

- [ ] **Step 5: Compile and run focused benchmarks**

Run:

```bash
cargo bench --bench router --no-run
cargo bench --bench router -- router_partition --sample-size 30
cargo bench --bench router -- router_enqueue --sample-size 30
cargo bench --bench router -- api_completion --sample-size 30
```

Expected: all complete and produce exact Criterion intervals. Record the allocation count and every interval for Task 7; do not compare them to history or worker benchmarks as if they measured the same boundary.

- [ ] **Step 6: Run router behavior and allocation tests**

Run:

```bash
cargo test --test router_allocations -- --nocapture --test-threads=1
cargo test --test router_api_boundary -- --nocapture
```

Expected: both pass.

- [ ] **Step 7: Commit**

```bash
git add src/router/partition.rs src/router.rs src/lib.rs benches/router.rs tests/router_allocations.rs Cargo.toml
git commit -m "bench: isolate router and api overhead"
```

---

### Task 7: Update documentation and run the complete verification gate

**Files:**
- Modify: `README.md`
- Create: `docs/superpowers/reports/2026-08-26-router-api-boundary-report.md`

**Interfaces:**
- Consumes: completed API behavior and exact Task 6 benchmark intervals.
- Produces: current architecture/performance documentation and final implementation evidence.

- [ ] **Step 1: Correct README architecture and API documentation**

Document:

- router as non-blocking partition/dispatch only;
- request-scoped channels owned by synchronous API calls;
- direct worker-to-API responses;
- `send -> Result<(), ContimeError>`;
- `apply -> Result<Vec<EventRejection>, ContimeError>`;
- rejection fields and reason codes;
- query, inspection, and advancement aggregation in `Contime`;
- worker/history ownership of identity, horizon, and memory admission.

Remove the previous `ApplyOutcome`, accepted-ID, and router-owned canonical identity descriptions.

- [ ] **Step 2: Record exact isolated benchmark evidence**

Add the hardware, OS, rustc version, profile, sample size, date, exact commands, allocation count, and Criterion `[low estimate high]` intervals. Clearly label router preparation, enqueue, API completion, worker, history, and end-to-end apply as different boundaries.

- [ ] **Step 3: Run formatting and strict linting**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both exit successfully with no warnings.

- [ ] **Step 4: Run all tests, doctests, and benchmark compilation**

Run:

```bash
cargo test --all-targets
cargo test --doc
cargo bench --bench apply --no-run
cargo bench --bench router --no-run
```

Expected: every command exits successfully. Record exact unit, integration, doctest, UI, and Criterion smoke counts.

- [ ] **Step 5: Prove the router boundary mechanically**

Run:

```bash
rg "\.recv\(|ApplyOutcome|InputRejection|accepted_input_ids|canonical_inputs|snapshot_ids\(\) -> Vec" src/router.rs src/api.rs src/worker.rs src/traits.rs contime_macros/src/lib.rs
```

Expected: no router receive, legacy outcome/identity symbols, or allocated snapshot-ID contract. Worker/API receives are permitted only in their intended files and should be inspected explicitly.

- [ ] **Step 6: Inspect scope and working tree**

Run:

```bash
git diff --check
git status --short
git diff --stat
```

Expected: only router/API boundary implementation, tests, benchmarks, README, and report changes.

- [ ] **Step 7: Write the implementation report**

Create `docs/superpowers/reports/2026-08-26-router-api-boundary-report.md` containing:

- task commit SHAs;
- RED/GREEN evidence;
- exact benchmark and allocation results;
- final verification counts;
- confirmation that the router never waits;
- confirmation that Timeless Runtime was not modified;
- deferred worker/history optimization work.

- [ ] **Step 8: Commit documentation and report**

```bash
git add README.md docs/superpowers/reports/2026-08-26-router-api-boundary-report.md
git commit -m "docs: report router api boundary correction"
```
