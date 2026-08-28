# Minimal Prepared Apply Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace ConTime's input path with one API-owned snapshot-batch map, move complete batches through the router and worker without avoidable cloning or temporary route collections, signal synchronous completion by channel disconnection, and measure the corrected warmed pipeline.

**Architecture:** The API consumes inputs into `PreparedRequest<IL>`, an `AHashMap` from snapshot ID to the final owned batch for that history. The router drains that map, hashes each snapshot ID once, and moves complete batches into one message per affected worker. A worker remains a plain blocking receive loop and directly applies each prepared batch; successful completion is the drop of the final request-scoped sender, while workers transmit only real rejections.

**Tech Stack:** Rust 2021, `ahash`, `crossbeam-channel`, Criterion, existing ConTime lane traits and test fixtures.

**Spec:** `docs/superpowers/specs/2026-08-28-minimal-prepared-apply-pipeline-design.md`

## Global Constraints

- Preserve the existing uncommitted `arcanex-updates` source and test changes as the accepted implementation baseline; do not reset, restore, or overwrite them.
- Before implementation commits, resolve whether the existing overlapping baseline may be committed. Do not silently include pre-existing changes in a task commit.
- Production workers use only a blocking `while let Ok(message) = receiver.recv()` loop; add no spin, polling, backoff, temperature, pinning, or scheduling layer.
- API, router, and worker execution order across independent snapshots is semantically irrelevant; histories retain canonical `(time, input_id)` ordering.
- One input routed to `N` snapshots performs exactly `N - 1` payload clones. Router and worker perform zero payload clones.
- Do not add `Arc<Input>` or a new channel library.
- Do not optimize Timeless Runtime or Spacetime in this plan.
- Write each behavioral test before its production change and record the expected RED failure.

## File Structure

- `src/batch.rs`: owns `PreparedRequest`, per-snapshot batch construction, conservative byte totals, memory-full rejection extraction, and doc-hidden test/benchmark inspection.
- `src/router/partition.rs`: consumes a prepared request into dense worker-index slots whose inner messages exist only for affected workers.
- `src/router.rs`: dispatches final worker messages and exposes benchmark adapters; never reopens individual inputs.
- `src/worker.rs`: owns the blocking worker loop, persistent history lookup, message memory reservation, direct batch application, and rejection-only completion sends.
- `src/api.rs`: constructs request-scoped completion channels, invokes preparation/dispatch, and collects rejection messages until disconnection.
- `src/lib.rs`: retains only the doc-hidden benchmark exports required by integration tests and Criterion.
- `tests/snapshot_batching.rs`: verifies API grouping, no temporary route-order contract, and exact clone counts.
- `src/router/tests.rs`: verifies prepared-batch partitioning and completion sender ownership.
- `tests/apply_context.rs`: verifies caller-managed asynchronous completion without waiting or explicit empty success values.
- `tests/router_api_boundary.rs`: verifies isolated concurrent synchronous completion and multi-worker closure semantics.
- `tests/apply_boundary_benchmarks.rs`: verifies production benchmark adapters before Criterion runs.
- `benches/apply_boundaries.rs`: matched warmed one-input and 1,000-input API/router/worker/history benchmarks.
- `benches/router.rs`: focused prepared-request partition, enqueue, and completion-by-close measurements.
- `README.md`: final pipeline contract, completion semantics, limitations, and fresh benchmark results.
- `docs/superpowers/reports/2026-08-28-minimal-prepared-apply-pipeline-report.md`: RED/GREEN evidence, verification commands, and measured results.

---

### Task 1: API-Owned Prepared Request

**Files:**
- Modify: `src/batch.rs:1-120`
- Modify: `tests/snapshot_batching.rs:1-78`
- Modify: `src/lib.rs` doc-hidden batch exports

**Interfaces:**
- Consumes: `Input::id`, `Input::time`, `Input::conservative_size`, and `InputRoute::visit_snapshot_ids` through `InputLanes<SL>`.
- Produces: `PreparedRequest<IL>`, `SnapshotInputBatch<IL>`, `prepare_inputs_by_snapshot::<SL, IL, I>(inputs) -> PreparedRequest<IL>`, `memory_full_rejections_for_request(&PreparedRequest<IL>) -> Vec<EventRejection>`, and benchmark inspection methods used by Tasks 2 and 4.

- [ ] **Step 1: Replace the order-based batching assertion with map-content and clone-count tests**

Add a clone-counted marker fixture to `tests/snapshot_batching.rs`:

```rust
#[derive(Debug)]
struct CloneCountedRoute {
    event_id: u128,
    time: i64,
    snapshot_ids: Vec<u128>,
    clones: Arc<AtomicUsize>,
}

impl Clone for CloneCountedRoute {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::Relaxed);
        Self {
            event_id: self.event_id,
            time: self.time,
            snapshot_ids: self.snapshot_ids.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}
```

Implement the existing `Input`, `Marker`, and `InputRoute` traits exactly as `RouteMarker` does, add it to the local `lanes!` marker list, and add these tests:

```rust
#[test]
fn api_grouping_builds_one_complete_batch_per_snapshot_without_order_semantics() {
    let grouped = SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([
        marker(1, [7, 3]),
        marker(2, [3, 9]),
        marker(3, [7]),
    ]);

    assert_eq!(grouped.get(&7), Some(&vec![1, 3]));
    assert_eq!(grouped.get(&3), Some(&vec![1, 2]));
    assert_eq!(grouped.get(&9), Some(&vec![2]));
    assert_eq!(grouped.len(), 3);
}

#[test]
fn one_route_moves_the_input_without_cloning() {
    let clones = Arc::new(AtomicUsize::new(0));
    let input = CloneCountedRoute { event_id: 1, time: 10, snapshot_ids: vec![7], clones: Arc::clone(&clones) };
    let grouped = SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([input.into()]);

    assert_eq!(grouped.get(&7), Some(&vec![1]));
    assert_eq!(clones.load(Ordering::Relaxed), 0);
}

#[test]
fn three_routes_clone_the_input_exactly_twice() {
    let clones = Arc::new(AtomicUsize::new(0));
    let input = CloneCountedRoute { event_id: 1, time: 10, snapshot_ids: vec![7, 11, 19], clones: Arc::clone(&clones) };
    let grouped = SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([input.into()]);

    assert_eq!(grouped.len(), 3);
    assert_eq!(clones.load(Ordering::Relaxed), 2);
}
```

- [ ] **Step 2: Run the focused tests and record RED**

Run:

```bash
cargo test --test snapshot_batching -- --nocapture
```

Expected: compilation fails because `SnapshotBatchBenchmark::group` still returns an ordered `Vec<(u128, Vec<u128>)>`, and the old route collection does not satisfy the new map interface.

- [ ] **Step 3: Implement `PreparedRequest` as the final API representation**

Replace the vector-plus-index-map representation in `src/batch.rs` with:

```rust
#[doc(hidden)]
pub struct PreparedRequest<IL> {
    pub(crate) snapshots: AHashMap<u128, SnapshotInputBatch<IL>>,
    pub(crate) conservative_bytes: u64,
}

#[doc(hidden)]
pub struct SnapshotInputBatch<IL> {
    pub(crate) inputs: Vec<IL>,
    pub(crate) conservative_bytes: u64,
}

pub(crate) fn prepare_inputs_by_snapshot<SL, IL, I>(inputs: I) -> PreparedRequest<IL>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    I: IntoIterator<Item = IL>,
{
    let mut snapshots = AHashMap::<u128, SnapshotInputBatch<IL>>::new();
    let mut total_bytes = 0_u64;

    for input in inputs {
        let route_bytes = input.conservative_size().saturating_add(RETAINED_ID_BYTES);
        let mut pending_snapshot_id = None;
        input.visit_snapshot_ids(&mut |snapshot_id| {
            if let Some(previous_snapshot_id) = pending_snapshot_id.replace(snapshot_id) {
                push_routed_input(&mut snapshots, previous_snapshot_id, input.clone(), route_bytes);
                total_bytes = total_bytes.saturating_add(route_bytes);
            }
        });
        if let Some(final_snapshot_id) = pending_snapshot_id {
            push_routed_input(&mut snapshots, final_snapshot_id, input, route_bytes);
            total_bytes = total_bytes.saturating_add(route_bytes);
        }
    }

    PreparedRequest { snapshots, conservative_bytes: total_bytes }
}
```

Make `push_routed_input` insert directly into the map value and append the owned input. Rename the API rejection extractor to `memory_full_rejections_for_request` and make it inspect this same map without constructing an ordered mirror. Update `SnapshotBatchBenchmark` the same way. Preserve the existing retained-byte calculation.

- [ ] **Step 4: Run focused tests and existing memory tests**

Run:

```bash
cargo test --test snapshot_batching -- --nocapture
cargo test --test memory -- --nocapture
```

Expected: all snapshot batching and memory tests pass; clone counts are exactly `0` and `2`.

- [ ] **Step 5: Commit the API preparation unit**

After the pre-existing dirty baseline is resolved for commits:

```bash
git add src/batch.rs src/lib.rs tests/snapshot_batching.rs
git commit -m "refactor: prepare snapshot batches in api map"
```

---

### Task 2: Router Partitioning and Direct Worker Application

**Files:**
- Modify: `src/router/partition.rs:1-74`
- Modify: `src/router.rs:12-225`
- Modify: `src/router/tests.rs:1-27`
- Modify: `src/worker.rs:1-260`
- Modify: `tests/apply_boundary_benchmarks.rs:53-133`
- Modify: `tests/router_allocations.rs`

**Interfaces:**
- Consumes: `PreparedRequest<IL>` and `SnapshotInputBatch<IL>` from Task 1.
- Produces: `RoutePartitioner::partition_prepared_request(PreparedRequest<IL>) -> Vec<Option<WorkerInputBatch<IL>>>`; `WorkerInputBatch<IL>` contains `Vec<(u128, SnapshotInputBatch<IL>)>` and a conservative byte total; `memory_full_rejections_for_worker(&[(u128, SnapshotInputBatch<IL>)]) -> Vec<EventRejection>`; `Router::dispatch_prepared_request` sends each affected worker message once.

- [ ] **Step 1: Write router tests against prepared requests and affected-only worker storage**

Update `src/router/tests.rs` so the input test prepares one request and asserts completion by worker ownership rather than vector order:

```rust
#[test]
fn dispatch_prepared_request_sends_one_message_per_affected_worker() {
    let mut router = Router::<TestSnapshotLanes, TestInputLanes>::new(8, 1_000_000);
    router.partitioner = RoutePartitioner::with_hasher(8, RandomState::with_seeds(1, 2, 3, 4));
    let first_snapshot_id = 1;
    let first_worker = router.worker_index(first_snapshot_id);
    let second_snapshot_id = (2..).find(|id| router.worker_index(*id) != first_worker).unwrap();
    let request = prepare_inputs_by_snapshot::<TestSnapshotLanes, TestInputLanes, _>([
        TestEvent::Positive(first_snapshot_id, 10, 100, 1).into(),
        TestEvent::Positive(second_snapshot_id, 10, 200, 1).into(),
    ]);
    let (completion_tx, completion_rx) = unbounded();

    router.dispatch_prepared_request(request, completion_tx).unwrap();
    assert!(completion_rx.iter().flatten().collect::<Vec<_>>().is_empty());
}
```

Update `tests/router_allocations.rs` to assert the partition adapter reports two affected messages for two worker hashes and never reports empty inner messages.

- [ ] **Step 2: Run router and boundary tests to record RED**

Run:

```bash
cargo test router::tests --lib -- --nocapture
cargo test --test router_allocations -- --nocapture
cargo test --test apply_boundary_benchmarks -- --nocapture
```

Expected: compilation fails because the router still consumes a vector of batches, returns an affected-worker count, and constructs an initialized inner vector for every worker.

- [ ] **Step 3: Partition the prepared map into affected-only worker messages**

Implement the partition structure in `src/router/partition.rs`:

```rust
#[doc(hidden)]
pub struct WorkerInputBatch<IL> {
    pub(crate) snapshot_batches: Vec<(u128, SnapshotInputBatch<IL>)>,
    pub(crate) conservative_bytes: u64,
}

pub(crate) fn partition_prepared_request<IL>(
    &self,
    request: PreparedRequest<IL>,
) -> Vec<Option<WorkerInputBatch<IL>>> {
    let mut workers = Vec::with_capacity(self.worker_count);
    workers.resize_with(self.worker_count, || None);

    for (snapshot_id, batch) in request.snapshots {
        let worker = workers[self.worker_index(snapshot_id)].get_or_insert_with(|| WorkerInputBatch {
            snapshot_batches: Vec::new(),
            conservative_bytes: 0,
        });
        worker.conservative_bytes = worker.conservative_bytes.saturating_add(batch.conservative_bytes);
        worker.snapshot_batches.push((snapshot_id, batch));
    }

    workers
}
```

Do not visit an input or call `InputRoute` in this module. Update the benchmark adapter to prepare through Task 1 and report only affected messages and total snapshot batches.

- [ ] **Step 4: Move complete batches through the router and directly into histories**

Change router dispatch to consume the prepared request, clone the completion sender once per affected worker, and move each `WorkerInputBatch` into `WorkerInbound::Inputs`.

Change the worker message to carry `Vec<(u128, SnapshotInputBatch<IL>)>`. In `src/worker.rs`, destructure each pair and use the map key for `history_by_id.entry(snapshot_id)`. Pass `batch.inputs` directly to `apply_routed_input_batch_with_memory`.

Update `stale_unseen_batch_rejections` to accept `snapshot_id` separately only if the ID is required by the call site; it must inspect the existing batch input slice without creating another input collection.

Add `memory_full_rejections_for_worker` beside the worker message type. It iterates the existing snapshot-batch input slices, collects their IDs only because a rejection result must own them, then sorts and deduplicates exactly as the current complete-message rejection path does.

- [ ] **Step 5: Run router, worker, boundary, and snapshot behavior tests**

Run:

```bash
cargo test router::tests --lib -- --nocapture
cargo test worker::tests --lib -- --nocapture
cargo test --test router_allocations -- --nocapture
cargo test --test apply_boundary_benchmarks -- --nocapture
cargo test --test router_api_boundary -- --nocapture
```

Expected: all focused tests pass; router test fixtures no longer rely on snapshot map order.

- [ ] **Step 6: Commit the router/worker ownership unit**

After the pre-existing dirty baseline is resolved for commits:

```bash
git add src/router.rs src/router/partition.rs src/router/tests.rs src/worker.rs tests/apply_boundary_benchmarks.rs tests/router_allocations.rs
git commit -m "refactor: move prepared batches through workers"
```

---

### Task 3: Completion by Sender Drop and Minimal Worker Loop

**Files:**
- Modify: `src/api.rs:1-28, 240-268`
- Modify: `src/router.rs:201-225`
- Modify: `src/worker.rs:1-91, 157-260`
- Modify: `tests/apply_context.rs:413-435`
- Modify: `tests/router_api_boundary.rs:73-125`
- Modify: `src/router/tests.rs:8-27`

**Interfaces:**
- Consumes: `Router::dispatch_prepared_request(request, completion_sender)` from Task 2.
- Produces: `Contime::send(inputs, completion: Sender<Vec<EventRejection>>) -> Result<(), ContimeError>`; `Contime::apply(inputs) -> Result<Vec<EventRejection>, ContimeError>` collects messages until sender disconnection; workers send only non-empty rejection vectors.

- [ ] **Step 1: Write completion-by-close tests**

Replace the current asynchronous explicit-empty-response assertion in `tests/apply_context.rs` with:

```rust
#[test]
fn send_returns_after_enqueue_and_success_is_completion_by_disconnect() {
    let (entered_tx, entered_rx) = flume::bounded(1);
    let (release_tx, release_rx) = flume::bounded(1);
    let applied = Arc::new(Mutex::new(Vec::new()));
    let contime = contime::Contime::<SnapshotLanes, InputLanes, BlockingApplyTrace>::new_with_apply_context(
        1,
        100_000,
        BlockingApplyTrace { entered_tx, release_rx, applied: Arc::clone(&applied) },
    );
    let (completion_tx, completion_rx) = crossbeam_channel::unbounded();

    contime.send(
        [OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 }].map(Into::into),
        completion_tx,
    ).unwrap();

    entered_rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
    assert!(applied.lock().unwrap().is_empty());
    assert_eq!(completion_rx.try_recv(), Err(crossbeam_channel::TryRecvError::Empty));
    release_tx.send(()).unwrap();
    assert_eq!(
        completion_rx.recv_timeout(std::time::Duration::from_secs(1)),
        Err(crossbeam_channel::RecvTimeoutError::Disconnected),
    );
    let snapshot = contime.query_at(11, &[3]).unwrap().pop().flatten().unwrap();
    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 11, value: 10 }));
}
```

Add a worker unit test proving non-empty rejections are sent before disconnect, and a successful completion sends no value before disconnect. Keep the existing concurrent-apply test in `tests/router_api_boundary.rs` as the isolation proof.

- [ ] **Step 2: Run completion tests and record RED**

Run:

```bash
cargo test --test apply_context send_returns_after_enqueue_and_success_is_completion_by_disconnect -- --nocapture
cargo test worker::tests --lib -- --nocapture
cargo test --test router_api_boundary -- --nocapture
```

Expected: the send signature still returns an affected count, successful workers explicitly send an empty vector, and the channel remains connected while the test's sender ownership does not follow the new contract.

- [ ] **Step 3: Make `send` consume the completion sender and return only dispatch success**

Implement:

```rust
pub fn send<I>(
    &self,
    inputs: I,
    completion: Sender<Vec<EventRejection>>,
) -> Result<(), ContimeError>
where
    I: IntoIterator<Item = IL>,
{
    let request = prepare_inputs_by_snapshot::<SL, IL, I>(inputs);
    if !self.memory.can_fit(request.conservative_bytes) {
        return Err(ContimeError::MemoryFull);
    }
    self.router.dispatch_prepared_request(request, completion).map_err(Into::into)
}
```

The router consumes the supplied sender, clones it only into affected worker messages, and lets its original drop on return. It returns `Result<(), RouterError>` rather than an affected-worker response count for input dispatch. Query and advancement counts remain unchanged because those operations still expect explicit responses.

- [ ] **Step 4: Make synchronous `apply` collect actual rejections until disconnection**

Implement a count-free collector:

```rust
fn collect_event_rejections(
    response_rx: &Receiver<Vec<EventRejection>>,
) -> Vec<EventRejection> {
    let mut rejections = Vec::new();
    for worker_rejections in response_rx {
        merge_event_rejections(&mut rejections, worker_rejections);
    }
    rejections
}
```

`apply` must prepare the request once, preserve API memory-full rejections from that request, create one unbounded channel, dispatch the prepared request with the sender, drop no additional retained sender, and return the collector result after disconnection. Do not implement `apply` by preparing the iterator once for memory checks and again through public `send`.

- [ ] **Step 5: Send only real rejection values and simplify worker lifetime**

Replace generic `complete` behavior with:

```rust
fn complete(completion: Sender<Vec<EventRejection>>, rejections: Vec<EventRejection>) {
    if !rejections.is_empty() {
        let _ = completion.send(rejections);
    }
}
```

Remove `AtomicBool`, `is_running`, and the outer relaxed-load condition from `Worker`. Retain explicit `Shutdown` and thread joining, with the thread body reduced to:

```rust
while let Ok(inbound) = worker_inbound_rx.recv() {
    match inbound {
        WorkerInbound::Shutdown => break,
        WorkerInbound::AdvanceTime { time: new_time, reply } => {
            current_time = new_time.clone();
            for history in history_by_id.values_mut() {
                let bytes_delta = history.advance_with_context(new_time.clone(), &mut apply_context);
                memory.apply_delta(bytes_delta);
            }
            let _ = reply.send(());
        }
        WorkerInbound::Inputs { snapshot_batches, conservative_bytes, completion } => {
            if !memory.try_reserve(conservative_bytes) {
                complete(completion, memory_full_rejections_for_worker(&snapshot_batches));
                continue;
            }
            let result = apply_snapshot_batches(
                snapshot_batches,
                &mut history_by_id,
                current_time.clone(),
                lower_time_horizon_delta.clone(),
                &mut apply_context,
                &memory,
            );
            memory.reconcile_reservation(conservative_bytes, result.actual_delta);
            complete(completion, result.rejections);
        }
        WorkerInbound::SnapshotsAt { snapshot_requests, time, reply } => {
            let results = snapshot_requests
                .into_iter()
                .map(|(position, snapshot_id)| {
                    let snapshot = history_by_id
                        .get(&snapshot_id)
                        .and_then(|history| history.snapshot_only_at_with_context(time.clone(), &mut apply_context));
                    (position, snapshot)
                })
                .collect();
            let _ = reply.send(results);
        }
    }
}
```

Extract the existing input-message body into `apply_snapshot_batches` returning a private `WorkerApplyResult { actual_delta: i64, rejections: Vec<EventRejection> }`; copy the current reservation-success semantics exactly and only move code out of the loop.

- [ ] **Step 6: Run API, completion, concurrency, and memory tests**

Run:

```bash
cargo test --test apply_context -- --nocapture
cargo test --test router_api_boundary -- --nocapture
cargo test --test memory -- --nocapture
cargo test router::tests --lib -- --nocapture
cargo test worker::tests --lib -- --nocapture
```

Expected: all focused tests pass; successful completion is observed only as `Disconnected`, real rejection messages remain available before disconnection, and concurrent calls remain isolated.

- [ ] **Step 7: Commit the completion/lifetime unit**

After the pre-existing dirty baseline is resolved for commits:

```bash
git add src/api.rs src/router.rs src/router/tests.rs src/worker.rs tests/apply_context.rs tests/router_api_boundary.rs
git commit -m "refactor: complete apply requests by sender drop"
```

---

### Task 4: Corrected Warmed Boundary Benchmarks

**Files:**
- Modify: `benches/apply_boundaries.rs:1-84`
- Modify: `benches/router.rs:60-115`
- Modify: `src/router.rs` benchmark adapter methods
- Modify: `src/worker.rs` benchmark adapter methods
- Modify: `src/api.rs` `CompletionBenchmark` adapter or remove it
- Modify: `tests/apply_boundary_benchmarks.rs:53-133`

**Interfaces:**
- Consumes: final API, router, worker, and history interfaces from Tasks 1-3.
- Produces: Criterion groups `apply_boundaries/1` and `apply_boundaries/1000`, each with `api`, `router`, `worker`, and `snapshot_history` measurements whose setup applies one real warm-up input.

- [ ] **Step 1: Add fixture tests proving real warm-up reaches each boundary**

Update `tests/apply_boundary_benchmarks.rs` with helpers that use warm-up ID `1`, time `1`, then measured IDs beginning at `2`, time `2`. Add assertions that the warm-up materializes the snapshot before the measured batch and that the measured query contains both warm-up and measured effects.

For one measured event, assert the final sum is `2`. For three measured events in the integration fixture, assert the final sum is `4`. The helpers must call a real apply method; an empty `query_at` is forbidden as a warm-up.

- [ ] **Step 2: Run the fixture tests and record RED**

Run:

```bash
cargo test --test apply_boundary_benchmarks -- --nocapture
```

Expected: existing adapters warm only through `advance_to`, and benchmark helper signatures cannot prepare/consume the new request and worker message types.

- [ ] **Step 3: Implement matched one-input and 1,000-input Criterion fixtures**

Restructure `benches/apply_boundaries.rs` around:

```rust
fn warm_input() -> BenchInputLanes {
    BenchEvent::Positive(SNAPSHOT_ID, 1, 1, 1).into()
}

fn measured_inputs(event_count: usize) -> Vec<BenchInputLanes> {
    (0..event_count)
        .map(|offset| BenchEvent::Positive(SNAPSHOT_ID, 2, 2 + offset as u128, 1).into())
        .collect()
}

fn benchmark_apply_boundaries(runner: &mut Criterion) {
    for event_count in [1_usize, 1_000] {
        let mut group = runner.benchmark_group(format!("apply_boundaries/{event_count}"));
        register_api_boundary(&mut group, event_count);
        register_router_boundary(&mut group, event_count);
        register_worker_boundary(&mut group, event_count);
        register_snapshot_history_boundary(&mut group, event_count);
        group.finish();
    }
}
```

Implement the four named registration helpers in this file. Each helper uses `iter_batched_ref`; its setup creates its own boundary fixture, applies `warm_input()`, and prepares `measured_inputs(event_count)`. For API timing, input-to-snapshot grouping remains inside the timed `Contime::apply` call, while construction of the input values is outside. For router timing, API preparation is outside. For worker timing, both API preparation and router partitioning are outside. For history timing, the snapshot batch itself is outside. Every fixture applies its real warm-up input before preparing the measured work.

- [ ] **Step 4: Replace the explicit-empty completion microbenchmark**

Change `benches/router.rs` so the completion benchmark creates a request-scoped channel, clones the sender `worker_count` times, drops each clone without sending, drops the original, and consumes the receiver until disconnection. Name it `completion_by_disconnect/{worker_count}`. Do not retain `empty_rejections` as a current benchmark.

- [ ] **Step 5: Run benchmark compilation, fixture tests, and focused measurements**

Run:

```bash
cargo test --test apply_boundary_benchmarks -- --nocapture
cargo bench --bench apply_boundaries --no-run
cargo bench --bench router --no-run
cargo bench --bench apply_boundaries -- apply_boundaries --sample-size 30
cargo bench --bench router -- completion_by_disconnect --sample-size 30
```

Expected: tests pass; Criterion reports eight warmed outside-in measurements plus completion-by-disconnection for `1`, `2`, and `8` workers.

- [ ] **Step 6: Commit the corrected benchmark unit**

After the pre-existing dirty baseline is resolved for commits:

```bash
git add benches/apply_boundaries.rs benches/router.rs src/api.rs src/router.rs src/worker.rs tests/apply_boundary_benchmarks.rs
git commit -m "bench: measure warmed prepared apply boundaries"
```

---

### Task 5: Documentation, Full Verification, and Performance Report

**Files:**
- Modify: `README.md` flow, completion, ordering, API examples, and performance sections
- Create: `docs/superpowers/reports/2026-08-28-minimal-prepared-apply-pipeline-report.md`
- Modify if required by compiler evidence: direct ConTime tests/examples using the changed `send` return type

**Interfaces:**
- Consumes: the completed production pipeline and Criterion results from Tasks 1-4.
- Produces: accurate public documentation, a reproducible verification report, and a clean ConTime test/benchmark gate.

- [ ] **Step 1: Update README contracts before inserting measurements**

Document this exact flow:

```text
API inputs -> AHashMap<snapshot ID, snapshot batch>
           -> router hashes each snapshot ID once
           -> one message per affected worker
           -> direct snapshot-history application
```

State that independent snapshot processing order is irrelevant, histories canonicalize by `(time, input ID)`, one-route inputs are moved without cloning, multi-route inputs clone exactly once per additional destination, and synchronous success is final-sender disconnection rather than an empty worker response.

Retain the existing warning about advisory memory checks and partial cross-worker application.

- [ ] **Step 2: Run formatting and focused tests**

Run:

```bash
cargo fmt --check
cargo test --test snapshot_batching
cargo test --test apply_context
cargo test --test router_api_boundary
cargo test --test apply_boundary_benchmarks
cargo test --test memory
```

Expected: every command passes with no ignored failure.

- [ ] **Step 3: Run strict lint and the complete target suite**

Run:

```bash
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Expected: strict Clippy passes and every unit, integration, doctest, example test, and Criterion smoke target passes.

- [ ] **Step 4: Re-run the release measurements used by README**

Run:

```bash
cargo bench --bench apply_boundaries -- apply_boundaries --sample-size 30
cargo bench --bench router -- completion_by_disconnect --sample-size 30
```

Copy Criterion's exact `[low estimate high]` intervals into README. For every boundary, report total time and per-input time for the 1,000-input case. Do not compare against stale Criterion change percentages; report the fresh absolute results.

- [ ] **Step 5: Write the implementation report**

Create `docs/superpowers/reports/2026-08-28-minimal-prepared-apply-pipeline-report.md`. Use the title `Minimal Prepared Apply Pipeline Report` and these sections in order: `Scope`, `RED evidence`, `GREEN evidence`, `Benchmarks`, and `Deferred work`. Under `Scope`, list the API grouping representation, router partition representation, worker direct-apply loop, completion-by-disconnection, and corrected warm-up fixtures. Under `RED evidence`, provide a two-column table containing every focused test and its captured pre-implementation failure. Under `GREEN evidence`, provide a two-column table containing every verification command and pass count. Under `Benchmarks`, provide columns for boundary, one-input interval, 1,000-input interval, and 1,000-input per-event point estimate. Under `Deferred work`, list cross-worker transactional memory admission, the Timeless Runtime benchmark update, and the Spacetime runtime benchmark rerun. Every table cell must contain captured evidence; omit no row and use no replacement marker.

- [ ] **Step 6: Commit documentation and the verified report**

After the pre-existing dirty baseline is resolved for commits:

```bash
git add README.md docs/superpowers/reports/2026-08-28-minimal-prepared-apply-pipeline-report.md
git commit -m "docs: report prepared apply performance"
```

- [ ] **Step 7: Record final repository state**

Run:

```bash
git status --short --branch
git log -5 --oneline
```

Expected: only explicitly preserved pre-existing changes remain; report every remaining path rather than describing the tree as clean.

## Self-Review

- Spec coverage: API ownership, exact clone count, order irrelevance, router hashing, affected-only messages, direct worker apply, sender-drop completion, simple blocking worker lifetime, memory accounting, corrected warm-up, benchmarks, docs, and deferred Runtime/Spacetime work each map to a task above.
- Placeholder scan: production, test, documentation, and report steps contain no replacement marker, `TBD`, `TODO`, omitted code body, or unspecified error-handling instruction.
- Type consistency: Tasks 1-5 consistently use `PreparedRequest<IL>`, `SnapshotInputBatch<IL>`, `WorkerInputBatch<IL>`, `prepare_inputs_by_snapshot`, `dispatch_prepared_request`, and count-free input completion.
