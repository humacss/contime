# Horizon Advancement and Memory Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add monotonic worker-local horizon advancement that preserves replayable state, prunes obsolete events and checkpoints, releases tracked memory, and rejects later pre-horizon events.

**Architecture:** The API emits one advance request, the router broadcasts it to every worker, and each worker computes and stores its own horizon. Event storage owns admission and event pruning; checkpoint storage materializes one complete pre-horizon replay anchor before pruning. Core connects the isolated contracts and exposes asynchronous and synchronous consumer methods.

**Tech Stack:** Rust 2021, `crossbeam-channel`, `ahash`, `criterion`, isolated ConTime subcrates.

**Spec:** `docs/superpowers/specs/2026-09-01-horizon-memory-release-design.md`

## Global Constraints

- Modify isolated subcrates and their documentation only; do not modify the root `contime` implementation used as reference.
- Keep subcrates isolated: each declares its own input/output traits, while `contime-core` implements adapters between them.
- Prune only values whose time is strictly less than the horizon; values exactly at the horizon remain.
- Advancement is monotonic per worker; equal and older requests are successful no-ops.
- Force ordinary replay only for a currently dirty history whose `dirty_time < horizon`.
- Materialize a complete pre-horizon checkpoint anchor before dropping its source events.
- Do not add snapshot compaction, global advancement barriers, dynamic dispatch, or explicit internal collection-capacity shrinking.
- Use existing tracked ownership; advancement must not return or manually reconcile a memory delta.
- Use test-driven development and stage only the exact files belonging to each task because the root worktree already contains unrelated changes.

---

## File Structure

New focused units:

- `crates/events/src/advance.rs`: event horizon state transition and pruning.
- `crates/checkpoints/src/advance.rs`: pre-horizon anchor materialization and checkpoint pruning.
- `crates/worker/src/advance.rs`: worker-local replay-before-prune orchestration.
- `crates/api/src/send_advance_to.rs`: asynchronous advance enqueue.
- `crates/api/src/advance_to.rs`: synchronous sender-closure wrapper.
- `crates/router/src/advance.rs`: advance broadcast to all worker queues.
- `crates/router/benches/advance.rs`: isolated multi-worker broadcast benchmark.
- `crates/core/src/advance.rs`: public core API and unit-level adapter exercise.
- `crates/core/tests/advance.rs`: complete functional pipeline coverage.
- `crates/core/benches/advance.rs`: end-to-end advancement and memory-release benchmarks.

Existing type and dispatch units change only to expose those operations:

- `types.rs` files own new traits, message structs, enum variants, and configuration fields.
- `lib.rs` files only declare modules and re-export public items.
- `route.rs`, `work.rs`, `message.rs`, `start.rs`, `history.rs`, `checkpoint.rs`, and `query.rs` connect the new focused units to existing paths.

### Task 1: Event Horizon Admission and Pruning

**Files:**
- Create: `crates/events/src/advance.rs`
- Modify: `crates/events/src/types.rs`
- Modify: `crates/events/src/history.rs`
- Modify: `crates/events/src/insert.rs`
- Modify: `crates/events/src/lib.rs`
- Modify: `crates/events/benches/events.rs`

**Interfaces:**
- Consumes: existing `Event`, `EventKey`, ordered `VecDeque`, late `BTreeMap`, and retained-ID index.
- Produces: `EventHistory::with_horizon`, `EventHistory::horizon`, `EventHistory::prune_before`, `PruneResult`, and `Insert::BeforeHorizon`.

- [ ] **Step 1: Write failing horizon-admission tests in `insert.rs`**

Add tests proving that an event before the active horizon is rejected before deduplication, an event exactly at the horizon is accepted, and an empty history created with a nonzero horizon enforces it:

```rust
#[test]
fn insertion_rejects_only_times_strictly_before_the_horizon() {
    let mut history = EventHistory::with_horizon(10);

    assert_eq!(history.insert(event(1, 9)), Insert::BeforeHorizon);
    assert_eq!(history.insert(event(2, 10)), Insert::Inserted);
    assert_eq!(history.len(), 1);
}

#[test]
fn horizon_rejection_precedes_duplicate_detection() {
    let mut history = EventHistory::with_horizon(0);
    assert_eq!(history.insert(event(1, 10)), Insert::Inserted);
    history.prune_before(&20);

    assert_eq!(history.insert(event(1, 10)), Insert::BeforeHorizon);
}
```

- [ ] **Step 2: Run the focused insertion tests and verify failure**

Run: `cargo test --manifest-path crates/events/Cargo.toml insert::tests -- --nocapture`

Expected: compilation fails because `with_horizon`, `prune_before`, and `BeforeHorizon` do not exist.

- [ ] **Step 3: Add horizon state and strict admission**

Extend `EventHistory` and `Insert` in `types.rs`:

```rust
pub struct EventHistory<E: Event> {
    pub(crate) ordered: VecDeque<(EventKey<E::Time>, E)>,
    pub(crate) late: BTreeMap<EventKey<E::Time>, E>,
    pub(crate) retained_ids: AHashSet<u128>,
    pub(crate) dirty_time: E::Time,
    pub(crate) horizon: E::Time,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PruneResult {
    pub removed_ordered: usize,
    pub removed_late: usize,
}

pub enum Insert {
    Inserted,
    Duplicate,
    BeforeHorizon,
}
```

Initialize `horizon` in both constructors, expose `with_horizon(horizon)` and `horizon()`, and make `insert` compare `event.time() < self.horizon` before mutating `retained_ids`.

- [ ] **Step 4: Write failing pruning tests in `advance.rs`**

Cover late-tree removal, deque-front removal, strict boundary retention, ID reuse after pruning, monotonic horizon changes, and dirty metadata after removal:

```rust
#[test]
fn pruning_removes_both_stores_and_forgets_only_removed_ids() {
    let mut history = EventHistory::with_horizon(0);
    history.insert(event(3, 30));
    history.insert(event(1, 10));
    history.insert(event(2, 20));

    let removed = history.prune_before(&20);

    assert_eq!(removed, PruneResult { removed_ordered: 0, removed_late: 1 });
    assert_eq!(history.iter().map(|(key, _)| key.time).collect::<Vec<_>>(), vec![20, 30]);
    assert_eq!(history.insert(event(1, 20)), Insert::Inserted);
}

#[test]
fn an_older_prune_request_is_a_no_op() {
    let mut history = EventHistory::with_horizon(0);
    history.insert(event(1, 10));
    history.prune_before(&20);

    assert_eq!(history.prune_before(&15), PruneResult { removed_ordered: 0, removed_late: 0 });
    assert_eq!(history.horizon(), &20);
}
```

- [ ] **Step 5: Implement event pruning in `advance.rs`**

Implement a single monotonic operation:

```rust
impl<E: Event> EventHistory<E> {
    pub fn prune_before(&mut self, horizon: &E::Time) -> PruneResult {
        if horizon <= &self.horizon {
            return PruneResult { removed_ordered: 0, removed_late: 0 };
        }
        self.horizon = horizon.clone();

        let boundary = EventKey { time: horizon.clone(), event_id: u128::MIN };
        let retained_late = self.late.split_off(&boundary);
        let removed_late = std::mem::replace(&mut self.late, retained_late);
        let removed_late_count = removed_late.len();
        for event in removed_late.into_values() {
            self.retained_ids.remove(&event.event_id());
        }

        let mut removed_ordered = 0;
        while self.ordered.front().is_some_and(|(key, _)| key.time < *horizon) {
            let (_, event) = self.ordered.pop_front().expect("front event exists");
            self.retained_ids.remove(&event.event_id());
            removed_ordered += 1;
        }
        if self.is_empty() {
            self.dirty_time = horizon.clone();
        }
        PruneResult { removed_ordered, removed_late: removed_late_count }
    }
}
```

- [ ] **Step 6: Run the events tests**

Run: `cargo test --manifest-path crates/events/Cargo.toml`

Expected: all event-history tests pass.

- [ ] **Step 7: Add and run event-pruning benchmarks**

Extend `crates/events/benches/events.rs` with Criterion cases for 1, 100, and 1,000 late-tree removals and a 1,000-event deque-front removal. Construct histories in `iter_batched`; measure only `prune_before` and set `Throughput::Elements(removed_count)`.

Run: `cargo bench --manifest-path crates/events/Cargo.toml --bench events -- pruning`

Expected: Criterion reports all pruning cases without test failures.

- [ ] **Step 8: Commit the event-store unit**

```bash
git add crates/events/src/advance.rs crates/events/src/types.rs crates/events/src/history.rs crates/events/src/insert.rs crates/events/src/lib.rs crates/events/benches/events.rs
git commit -m "Add event horizon pruning"
```

### Task 2: Checkpoint Replay Anchor and Best-Effort Querying

**Files:**
- Create: `crates/checkpoints/src/advance.rs`
- Modify: `crates/checkpoints/src/types.rs`
- Modify: `crates/checkpoints/src/checkpoints.rs`
- Modify: `crates/checkpoints/src/replay.rs`
- Modify: `crates/checkpoints/src/query.rs`
- Modify: `crates/checkpoints/src/lib.rs`
- Modify: `crates/checkpoints/benches/query.rs`

**Interfaces:**
- Consumes: existing `Events`, `ApplyWrapper`, canonical event buckets, and `CheckpointStore` replay machinery.
- Produces: `ReplayAnchor<S>`, `CheckpointStore::anchor`, and `advance_before(&mut CheckpointStore, &Events, &mut ApplyWrapper, &Time)`.

- [ ] **Step 1: Write failing tests for initial and materialized anchors**

Add tests in `replay.rs` and `advance.rs` proving that first materialization retains clean initial state and that advancing across events between cadence checkpoints creates a complete anchor:

```rust
#[test]
fn first_materialization_retains_the_clean_initial_snapshot() {
    let mut events = TestEvents::new(0, vec![event(1, 10, 5)]);
    let mut store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });

    replay(&mut store, &mut events, &mut ());

    let anchor = store.anchor().expect("materialized history has an anchor");
    assert_eq!(anchor.boundary, None);
    assert_eq!(anchor.snapshot.sum, 0);
}

#[test]
fn advance_folds_events_after_the_previous_checkpoint_into_the_anchor() {
    let mut events = TestEvents::new(0, (1..=100).map(|time| event(time, time as i64, 1)).collect());
    let mut store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 50 });
    replay(&mut store, &mut events, &mut ());

    advance_before(&mut store, &events, &mut (), &75);

    let anchor = store.anchor().unwrap();
    assert_eq!(anchor.boundary.as_ref().unwrap().time, 74);
    assert_eq!(anchor.snapshot.sum, 74);
}
```

- [ ] **Step 2: Run focused checkpoint tests and verify failure**

Run: `cargo test --manifest-path crates/checkpoints/Cargo.toml advance::tests replay::tests -- --nocapture`

Expected: compilation fails because replay anchors and `advance_before` are absent.

- [ ] **Step 3: Add a separate replay-anchor representation**

Define the anchor separately from cadence checkpoints so a clean initial snapshot can have no event boundary:

```rust
#[derive(Clone, Debug)]
pub struct ReplayAnchor<S: Snapshot> {
    pub boundary: Option<CheckpointKey<S::Time>>,
    pub snapshot: S,
    pub history_event_count: u64,
}

pub struct CheckpointStore<S: Snapshot> {
    pub(crate) snapshot_id: u128,
    pub(crate) interval: u64,
    pub(crate) anchor: Option<ReplayAnchor<S>>,
    pub(crate) checkpoints: VecDeque<Checkpoint<S>>,
}
```

Expose `anchor()`. Update `current`, replay-base selection, iteration used by memory accounting, and first materialization so `S::create(snapshot_id, first_event)` is cloned into `anchor` before the first bucket mutates the working snapshot.

- [ ] **Step 4: Implement exclusive pre-horizon anchor materialization**

In `advance.rs`, clone the closest retained base, apply canonical buckets with `event.time < horizon`, retain the last applied `CheckpointKey`, and replace `store.anchor`. Then pop every cadence checkpoint whose key time is strictly below the horizon. Return an `AdvanceResult` containing only diagnostic counts:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdvanceResult {
    pub removed_checkpoints: usize,
}

pub fn advance_before<H, S, W, E, T>(
    store: &mut CheckpointStore<S>,
    events: &H,
    wrapper: &mut W,
    horizon: &T,
) -> AdvanceResult
where
    H: Events<Time = T, Event = E>,
    S: ApplyEvents<E> + Snapshot<Time = T>,
    W: ApplyWrapper<S, E>,
    T: Clone + Default + Ord;
```

Use the existing same-timestamp bucketing and `apply` function. Do not acknowledge event history from this function; worker orchestration performs ordinary replay first only when required.

- [ ] **Step 5: Make queries use the retained anchor as best effort**

Modify `query_at` so the anchor is the default working snapshot when no cadence checkpoint exists at or before the requested time. When the requested time predates the anchor boundary, return the cloned anchor without attempting to reverse it. For newer queries, iterate retained events strictly after the anchor boundary and through the requested time.

Add this regression test:

```rust
#[test]
fn a_query_older_than_the_retained_anchor_returns_the_anchor() {
    let mut events = TestEvents::new(0, vec![event(1, 10, 1), event(2, 20, 2)]);
    let mut store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });
    replay(&mut store, &mut events, &mut ());
    advance_before(&mut store, &events, &mut (), &20);

    let snapshot = query_at(&store, &events, &mut (), 5).unwrap();
    assert_eq!(snapshot.sum, 1);
}
```

- [ ] **Step 6: Run checkpoint tests**

Run: `cargo test --manifest-path crates/checkpoints/Cargo.toml`

Expected: replay, query, checkpoint, and advancement tests pass.

- [ ] **Step 7: Benchmark anchor materialization**

Extend the checkpoint benchmark with a checkpoint-aligned horizon and a horizon midway between checkpoints over 1,000 events. Measure `advance_before` only; fixtures perform initial replay outside timing.

Run: `cargo bench --manifest-path crates/checkpoints/Cargo.toml --bench query -- advance`

Expected: Criterion reports both anchor paths.

- [ ] **Step 8: Commit checkpoint advancement**

```bash
git add crates/checkpoints/src/advance.rs crates/checkpoints/src/types.rs crates/checkpoints/src/checkpoints.rs crates/checkpoints/src/replay.rs crates/checkpoints/src/query.rs crates/checkpoints/src/lib.rs crates/checkpoints/benches/query.rs
git commit -m "Add checkpoint horizon anchors"
```

### Task 3: Worker-Local Advancement

**Files:**
- Create: `crates/worker/src/advance.rs`
- Modify: `crates/worker/src/types.rs`
- Modify: `crates/worker/src/schedule.rs`
- Modify: `crates/worker/src/events.rs`
- Modify: `crates/worker/src/work.rs`
- Modify: `crates/worker/src/query.rs`
- Modify: `crates/worker/src/lib.rs`
- Modify: `crates/worker/benches/worker_settings.rs`

**Interfaces:**
- Consumes: event-store `dirty_time`/prune contract, checkpoint update/advance contract, and worker scheduling state.
- Produces: `AdvanceTime`, `AdvanceInput`, `AdvanceOutput`, expanded `Events`/`Checkpoints` contracts, and `WorkInputKind::Advance`.

- [ ] **Step 1: Write failing scheduler extraction tests**

Add `Schedule::take(snapshot_id) -> bool`, with lazy stale-entry cleanup preserved:

```rust
#[test]
fn taking_one_dirty_snapshot_deactivates_only_that_snapshot() {
    let now = Instant::now();
    let mut schedule = Schedule::new(usize::MAX, 2);
    schedule.mark_dirty(7, now);
    schedule.mark_dirty(9, now);

    assert!(schedule.take(7));
    assert!(!schedule.take(7));
    assert_eq!(schedule.pop_largest(now), Some(9));
}
```

- [ ] **Step 2: Implement and verify `Schedule::take`**

Set the selected state's `actual_count` to zero, decrement `active_count` once, and leave heap/deadline entries to existing stale cleanup.

Run: `cargo test --manifest-path crates/worker/Cargo.toml schedule::tests::taking_one_dirty_snapshot_deactivates_only_that_snapshot`

Expected: PASS.

- [ ] **Step 3: Define isolated advancement contracts**

Add to `types.rs`:

```rust
pub trait AdvanceTime: Clone + Default + Ord {
    fn saturating_sub(&self, retention: &Self) -> Self;
}

pub trait AdvanceInput {
    type Time: AdvanceTime;
    type Completion;
    fn into_parts(self) -> (Self::Time, Self::Completion);
}

pub trait AdvanceOutput<T, C>: Sized {
    fn advance(time: T, completion: C) -> Self;
}
```

Implement `AdvanceTime` for Rust integer timestamp types with their inherent saturating subtraction. Extend worker `Events` with associated `Time`, `create(snapshot_id, config, horizon)`, `dirty_time`, and `prune_before`. Extend `Checkpoints` with `advance_before(events, context, horizon)`. Add `Advance` to `WorkInputKind` and `WorkInput`.

- [ ] **Step 4: Write failing worker advancement tests**

Use stub stores that record calls. Cover monotonic no-op, forced replay for dirty time below horizon, no forced replay at the horizon, prune ordering, new-history initialization with the active horizon, and completion-handle drop:

```rust
#[test]
fn advance_replays_before_anchor_and_event_pruning() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut fixture = worker_fixture(Arc::clone(&log), dirty_history(5));

    advance_worker(&mut fixture, 100, 90);

    assert_eq!(*log.lock().unwrap(), vec!["replay", "anchor", "events"]);
}

#[test]
fn dirty_state_at_the_horizon_remains_scheduled() {
    let mut fixture = worker_fixture(Default::default(), dirty_history(10));
    advance_worker(&mut fixture, 20, 10);
    assert!(fixture.schedule.take(7));
}
```

- [ ] **Step 5: Implement `advance.rs` and dispatch it from `work_messages`**

Maintain `current_time` and `horizon` locals in `work_messages`. On `Advance`:

```rust
if target_time > current_time {
    current_time = target_time;
    horizon = current_time.saturating_sub(&retention);
    let replay_ids = snapshots.iter()
        .filter(|(id, slot)| schedule.is_dirty(**id) && slot.events.dirty_time() < &horizon)
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for snapshot_id in replay_ids {
        schedule.take(snapshot_id);
        update_snapshot(snapshot_id, snapshots, checkpoints_config, checkpoints_context);
    }
    for slot in snapshots.values_mut() {
        if let Some(checkpoints) = slot.checkpoints.as_mut() {
            checkpoints.advance_before(&slot.events, checkpoints_context, &horizon);
        }
        slot.events.prune_before(&horizon);
    }
}
drop(completion);
```

Provide `Schedule::is_dirty`. Initialize histories created after advancement with the current horizon. Preserve apply waiter completion when forced replay calls `update_snapshot`.

- [ ] **Step 6: Run all worker tests**

Run: `cargo test --manifest-path crates/worker/Cargo.toml`

Expected: existing scheduling/apply/query tests and new advancement tests pass.

- [ ] **Step 7: Add worker benchmarks**

Add unit/bench cases for 1,000 clean histories, 1,000 histories requiring anchor pruning, and 1,000 dirty histories requiring replay. Reuse initialized maps and measure the advancement operation, not fixture creation.

Run: `cargo bench --manifest-path crates/worker/Cargo.toml --bench worker_settings -- advance`

Expected: all three worker advancement cases report.

- [ ] **Step 8: Commit worker advancement**

```bash
git add crates/worker/src/advance.rs crates/worker/src/types.rs crates/worker/src/schedule.rs crates/worker/src/events.rs crates/worker/src/work.rs crates/worker/src/query.rs crates/worker/src/lib.rs crates/worker/benches/worker_settings.rs
git commit -m "Add worker horizon advancement"
```

### Task 4: API Advance Functions

**Files:**
- Create: `crates/api/src/send_advance_to.rs`
- Create: `crates/api/src/advance_to.rs`
- Modify: `crates/api/src/types.rs`
- Modify: `crates/api/src/lib.rs`

**Interfaces:**
- Consumes: a generic downstream `Sender<O>`.
- Produces: `AdvanceOutput<T>`, `send_advance_to`, and `advance_to`.

- [ ] **Step 1: Write failing asynchronous API tests**

Define an adapter message and verify exact forwarding and closed-channel errors:

```rust
struct AdvanceMessage {
    time: u64,
    completion: Sender<()>,
}

impl AdvanceOutput<u64> for AdvanceMessage {
    fn advance(time: u64, completion: Sender<()>) -> Self {
        Self { time, completion }
    }
}

#[test]
fn send_advance_forwards_time_and_completion() {
    let (output, input) = unbounded();
    let (completion, done) = unbounded();
    send_advance_to::<AdvanceMessage, _>(&output, 50, completion).unwrap();
    let message = input.recv().unwrap();
    assert_eq!(message.time, 50);
    drop(message.completion);
    assert_eq!(done.try_recv(), Err(TryRecvError::Disconnected));
}
```

- [ ] **Step 2: Implement `AdvanceOutput` and `send_advance_to`**

Use the same file-local `Deps` stubbing pattern as `send.rs`. The public signature is:

```rust
pub fn send_advance_to<O, T>(
    output: &Sender<O>,
    time: T,
    completion: Sender<()>,
) -> Result<(), ApiError>
where
    O: AdvanceOutput<T>;
```

- [ ] **Step 3: Write failing synchronous wrapper tests**

Stub the dependency so it drops the supplied sender immediately and verify the wrapper blocks until all clones are dropped. Use a second test where a clone remains live until a helper thread releases it.

- [ ] **Step 4: Implement `advance_to`**

```rust
pub fn advance_to<O, T>(output: &Sender<O>, time: T) -> Result<(), ApiError>
where
    O: AdvanceOutput<T>,
{
    let (completion, receiver) = crossbeam_channel::unbounded();
    send_advance_to(output, time, completion)?;
    receiver.into_iter().for_each(drop);
    Ok(())
}
```

- [ ] **Step 5: Add inline Criterion sanity benchmarks and run tests**

Benchmark one asynchronous enqueue and one synchronous immediately-closing dependency. Keep channel construction outside the asynchronous measured region.

Run: `cargo test --manifest-path crates/api/Cargo.toml`

Expected: all API apply/query/advance tests pass.

- [ ] **Step 6: Commit API advancement**

```bash
git add crates/api/src/send_advance_to.rs crates/api/src/advance_to.rs crates/api/src/types.rs crates/api/src/lib.rs
git commit -m "Add horizon advance API"
```

### Task 5: Router Advance Broadcast

**Files:**
- Create: `crates/router/src/advance.rs`
- Modify: `crates/router/src/types.rs`
- Modify: `crates/router/src/route.rs`
- Modify: `crates/router/src/query.rs`
- Modify: `crates/router/src/lib.rs`
- Create: `crates/router/benches/advance.rs`
- Modify: `crates/router/Cargo.toml`

**Interfaces:**
- Consumes: `AdvanceInput` from the caller-selected router message.
- Produces: one `AdvanceWorkerOutput` per worker, moving the final completion handle and cloning it only for preceding workers.

- [ ] **Step 1: Write failing broadcast tests**

Cover zero workers, one worker with no clone, four workers receiving identical time, worker-send failure, and completion closure only after all worker messages drop:

```rust
#[test]
fn advance_is_broadcast_to_every_worker() {
    let (completion, done) = unbounded();
    let workers = worker_channels(4);

    route_advance(Advance { time: 50, completion }, &workers.senders).unwrap();

    assert_eq!(workers.receivers.iter().map(|rx| rx.recv().unwrap().time).collect::<Vec<_>>(), vec![50; 4]);
    drop(workers);
    assert_eq!(done.try_recv(), Err(TryRecvError::Disconnected));
}
```

- [ ] **Step 2: Define router contracts and implement broadcast**

Add:

```rust
pub trait AdvanceInput {
    type Time: Clone;
    type Completion: Clone;
    fn into_parts(self) -> (Self::Time, Self::Completion);
}

pub trait AdvanceWorkerOutput<T, C>: Sized {
    fn advance(time: T, completion: C) -> Self;
}
```

`route_advance` validates nonempty workers, clones time and completion for all but the final worker, and maps send failure to `RouterError::WorkerUnavailable`.

- [ ] **Step 3: Extend unified router dispatch**

Add `Advance` to `RouteInputKind`/`RouteInput`, extend `route_messages` bounds, and dispatch to `route_advance`. Do not hash or allocate route vectors for advancement.

- [ ] **Step 4: Run router tests and benchmark**

Run: `cargo test --manifest-path crates/router/Cargo.toml`

Add `benches/advance.rs` and its explicit Cargo bench target. Benchmark broadcast to 1, 4, and 10 workers with pre-created channels.

Run: `cargo bench --manifest-path crates/router/Cargo.toml --bench advance`

Expected: router tests pass and broadcast measurements report.

- [ ] **Step 5: Commit router advancement**

```bash
git add crates/router/src/advance.rs crates/router/src/types.rs crates/router/src/route.rs crates/router/src/query.rs crates/router/src/lib.rs crates/router/benches/advance.rs crates/router/Cargo.toml
git commit -m "Broadcast horizon advances"
```

### Task 6: Compose Advancement in Core

**Files:**
- Create: `crates/core/src/advance.rs`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/message.rs`
- Modify: `crates/core/src/history.rs`
- Modify: `crates/core/src/checkpoint.rs`
- Modify: `crates/core/src/worker.rs`
- Modify: `crates/core/src/start.rs`
- Modify: `crates/core/src/query.rs`
- Modify: `crates/core/src/apply.rs`
- Modify: `crates/core/src/send.rs`
- Modify: `crates/core/src/shutdown.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/tests/query.rs`
- Modify: `crates/core/benches/apply.rs`
- Modify: `crates/core/benches/query.rs`

**Interfaces:**
- Consumes: API, router, worker, event, checkpoint, runtime, and memory contracts completed above.
- Produces: `ConTimeConfig<T>::history_retention`, `ConTime::send_advance_to`, `ConTime::advance_to`, `RejectionReason::BeforeHistoryHorizon`, and concrete advance message adapters.

- [ ] **Step 1: Write failing core adapter tests in `message.rs`**

Define one core-owned message type used on both boundaries:

```rust
pub struct Advance<T> {
    pub(crate) time: T,
    pub(crate) completion: Sender<()>,
}
```

Write tests that construct `RouterMessage::Advance` through `contime_api::AdvanceOutput`, destructure it through `contime_router::AdvanceInput`, construct `WorkerMessage::Advance` through `AdvanceWorkerOutput`, and destructure it through `contime_worker::AdvanceInput` without translating the timestamp or sender.

- [ ] **Step 2: Add concrete message adapters and enum variants**

Implement the adjacent traits on `Advance<T>`, `RouterMessage<I, S>`, and `WorkerMessage<I, S>`. Expand every unified kind match exhaustively. Use `Sender<()>` directly; do not add an acknowledgement payload or counter.

- [ ] **Step 3: Add time and configuration wiring**

Change configuration to:

```rust
pub struct ConTimeConfig<T> {
    pub router_count: usize,
    pub worker_count: usize,
    pub router_seed: u64,
    pub memory_limit: usize,
    pub memory_buffer: usize,
    pub history_retention: T,
    pub worker: contime_worker::WorkerConfig,
    pub checkpoints: contime_checkpoints::CheckpointConfig,
}
```

Require `I::Time: contime_worker::AdvanceTime` for `ConTime::start`, store the retention value in every `WorkerProcess`, and pass it to `work_messages`. Update existing core test/benchmark configurations with explicit retention values that preserve their previous fixtures.

- [ ] **Step 4: Adapt core event storage and rejection mapping**

Implement the expanded worker `Events` trait for `History<I>` using `EventHistory::with_horizon`. Map `Insert::BeforeHorizon` to:

```rust
RejectionMessage {
    event_id: input.event_id(),
    reason: RejectionReason::BeforeHistoryHorizon,
}
```

Return `changed: false` for rejected events and retain no tracked handle in history. Implement `dirty_time` and `prune_before` by delegation.

Change `History<I>`'s worker rejection type to `RejectionMessage<RejectionReason>` and replace the infallible completion adapter with:

```rust
impl Completion<RejectionMessage<RejectionReason>> for CompletionHandle {
    fn reject(self, rejections: Vec<RejectionMessage<RejectionReason>>) {
        for rejection in rejections {
            let _ = self.sender.send(rejection);
        }
    }
}
```

Dropping `CompletionHandle` without calling `reject` continues to signal a successful worker result.

- [ ] **Step 5: Adapt checkpoint advancement with tracked size deltas**

Implement worker checkpoint advancement inside one tracked-box mutation:

```rust
fn advance_before(&mut self, events: &History<I>, context: &mut W, horizon: &I::Time) {
    self.state.update(|state| {
        contime_checkpoints::advance_before(&mut state.checkpoints, events, context, horizon);
    });
}
```

Update `CheckpointState::conservative_tracked_size` to include the separate replay anchor exactly once. Its existing `TrackedSizeDelta` implementation then reports checkpoint memory decreases automatically.

- [ ] **Step 6: Implement public core API in `advance.rs`**

```rust
impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    pub fn send_advance_to(&self, time: I::Time, completion: Sender<()>) -> Result<(), ApiError> {
        contime_api::send_advance_to(self.runtime.input(), time, completion)
    }

    pub fn advance_to(&self, time: I::Time) -> Result<(), ApiError> {
        contime_api::advance_to(self.runtime.input(), time)
    }
}
```

Add focused unit tests for forwarding, sender closure, and no memory changes in the API adapter itself.

- [ ] **Step 7: Update query behavior and run core unit tests**

Ensure the core checkpoint adapter delegates the checkpoint crate's best-effort anchor query unchanged. Add a core-level unit assertion that a query older than the retained anchor returns a snapshot.

Run: `cargo test --manifest-path crates/core/Cargo.toml --lib`

Expected: all existing core unit tests plus advancement unit tests pass.

- [ ] **Step 8: Commit core composition**

```bash
git add crates/core/src/advance.rs crates/core/src/types.rs crates/core/src/message.rs crates/core/src/history.rs crates/core/src/checkpoint.rs crates/core/src/worker.rs crates/core/src/start.rs crates/core/src/query.rs crates/core/src/apply.rs crates/core/src/send.rs crates/core/src/shutdown.rs crates/core/src/lib.rs crates/core/tests/query.rs crates/core/benches/apply.rs crates/core/benches/query.rs
git commit -m "Compose horizon advancement in core"
```

### Task 7: End-to-End Tests, Benchmarks, and Documentation

**Files:**
- Create: `crates/core/tests/advance.rs`
- Create: `crates/core/benches/advance.rs`
- Modify: `crates/core/Cargo.toml`
- Modify: `crates/events/README.md`
- Modify: `crates/checkpoints/README.md`
- Modify: `crates/worker/README.md`
- Modify: `crates/api/README.md`
- Modify: `crates/router/README.md`
- Modify: `crates/core/README.md`

**Interfaces:**
- Consumes: complete `ConTime` apply/query/advance pipeline.
- Produces: behavioral proof, comparative throughput, released-byte measurements, and user-facing contract documentation.

- [ ] **Step 1: Write end-to-end functional tests**

Create fixtures with timestamped events and snapshots that expose accumulated values. Include these tests:

```rust
#[test]
fn advance_preserves_state_releases_memory_and_rejects_late_old_events() {
    let contime = started(1, 1, 10);
    contime.apply(events_at([1, 5, 10, 15])).unwrap();
    let before = contime.used_memory();

    contime.advance_to(20).unwrap(); // horizon = 10

    assert_eq!(contime.query_at(20, [7]).unwrap()[0].value, 4);
    assert!(contime.used_memory() < before);
    let rejected = contime.apply([event(99, 9, 1)]).unwrap();
    assert_eq!(rejected[0].reason, RejectionReason::BeforeHistoryHorizon);
    assert!(contime.apply([event(100, 10, 1)]).unwrap().is_empty());
}

#[test]
fn asynchronous_advance_closes_after_every_worker_finishes() {
    let contime = started(2, 4, 10);
    let (completion, done) = unbounded();
    contime.send_advance_to(20, completion).unwrap();
    assert_eq!(done.into_iter().collect::<Vec<_>>(), Vec::<()>::new());
}
```

Also cover repeated and backward no-ops, ID reuse after pruning, a dirty pre-horizon event arriving before the advance message, a dirty event at the horizon remaining replayable, a history first seen after advancement, multi-worker fan-out, and best-effort pre-anchor query.

- [ ] **Step 2: Run core integration tests**

Run: `cargo test --manifest-path crates/core/Cargo.toml --test advance -- --nocapture`

Expected: all horizon and memory assertions pass.

- [ ] **Step 3: Add end-to-end Criterion benchmarks**

Register `advance` in `crates/core/Cargo.toml`. Benchmark warmed runtimes for topologies `(1 router, 1 worker)`, `(1, 4)`, `(1, 10)`, and `(2, 10)`. For each topology measure:

- 1,000 clean histories requiring only pruning;
- 1,000 histories whose anchor falls between cadence checkpoints;
- 1,000 dirty histories requiring replay before pruning.

Construct and apply fixtures outside timing. Record `used_memory` before and after each measured advance and assert the latter is smaller. Set `Throughput::Elements(1_000)`.

- [ ] **Step 4: Run advancement benchmarks and capture results**

Run: `cargo bench --manifest-path crates/core/Cargo.toml --bench advance`

Expected: all topology/workload combinations report time, elements per second, and satisfy memory-release assertions.

- [ ] **Step 5: Update crate READMEs**

Document:

- asynchronous sender-closure and synchronous `advance_to` behavior;
- strict horizon boundary and monotonic worker-local state;
- replay-before-prune and complete replay-anchor semantics;
- event-ID forgetting and `BeforeHistoryHorizon` rejection;
- best-effort queries older than the anchor;
- tracked memory release and excluded internal spare-capacity accounting;
- the measured benchmark tables with time per 1,000 histories, histories per second, topology, and released bytes.

Remove statements saying advancement is deferred or that affected crates are apply/query-only.

- [ ] **Step 6: Run all isolated crate verification**

Run each command separately:

```bash
cargo fmt --manifest-path crates/api/Cargo.toml -- --check
cargo test --manifest-path crates/api/Cargo.toml
cargo test --manifest-path crates/router/Cargo.toml
cargo test --manifest-path crates/events/Cargo.toml
cargo test --manifest-path crates/checkpoints/Cargo.toml
cargo test --manifest-path crates/worker/Cargo.toml
cargo test --manifest-path crates/runtime/Cargo.toml
cargo test --manifest-path crates/core/Cargo.toml
cargo check --manifest-path crates/core/Cargo.toml --all-targets
git diff --check -- crates/api crates/router crates/events crates/checkpoints crates/worker crates/runtime crates/core
```

Expected: every command exits successfully. Do not claim the dirty root crate was verified.

- [ ] **Step 7: Commit integration coverage and documentation**

```bash
git add crates/core/tests/advance.rs crates/core/benches/advance.rs crates/core/Cargo.toml crates/events/README.md crates/checkpoints/README.md crates/worker/README.md crates/api/README.md crates/router/README.md crates/core/README.md
git commit -m "Test and document horizon advancement"
```

- [ ] **Step 8: Review final scope**

Run: `git status --short`

Expected: the horizon commits contain only isolated subcrate and documentation files. Existing unrelated root modifications and untracked historical plan/report files remain unstaged and unchanged.
