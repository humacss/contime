# Hybrid History Storage Design

## Summary

ConTime currently stores every routed input in a per-snapshot
`BTreeMap<ContimeKey<Time>, Input>`. That representation gives deterministic
middle insertion and range traversal, but it makes the overwhelmingly common
ordered path pay for a tree lookup and allocation for every event.

Replace that single collection with a hybrid history:

- a contiguous append history for inputs that arrive in canonical order;
- a `BTreeMap` containing only genuinely late inputs;
- a merge iterator that exposes both collections as one canonical
  `(time, input_id)` sequence.

Global input identity remains independent of time. The router uses a retained
ID set for idempotency and a separate time-ordered retention index only to know
which IDs to forget when the horizon advances.

This is an internal storage change. High-level apply, query, inspection,
snapshot, checkpoint, marker, and horizon semantics remain unchanged. The
concrete type of the advanced `LocalSnapshotHistory::inputs` field necessarily
changes because exposing the current `BTreeMap` would defeat the new storage
boundary.

## Goals

- Make ordered admission a linear scan followed by contiguous appends.
- Preserve deterministic ordering by complete time and input ID.
- Preserve correct late-input replay and checkpoint invalidation.
- Keep horizon pruning and memory use bounded.
- Treat a retained input ID as a no-op regardless of the timestamp or payload
  on a repeated submission.
- Forget an input's identity when the input itself is removed beyond the
  horizon.
- Measure storage, routing, worker wake-up, and end-to-end costs separately.

## Non-goals

- Changing ConTime's high-level `Contime` API or event model.
- Optimizing Timeless Runtime in this pass.
- Replacing checkpoints or changing checkpoint cadence.
- Changing worker routing, worker ownership, or concurrency semantics.
- Making horizon advancement and concurrent multi-worker application
  transactional.
- Adding persistence for pruned IDs or inputs.

## Baseline and Motivation

Diagnostic Criterion measurements on the current branch established the
following approximate costs:

| Operation | 1 input | 100 inputs | 1,000 inputs |
| --- | ---: | ---: | ---: |
| Direct snapshot callback | 2.2 ns | 24 ns | 214 ns |
| Per-snapshot history admission and replay | 191 ns | 4.05 us | 48.6 us |
| Persistent asynchronous `send`, same snapshot | 672 ns | 25 us | 246 us |
| Persistent synchronous apply, same snapshot | 7.7 us | 69 us | 430 us |

The callback is already cheap. The direct history benchmark shows that
per-input ordered insertion into the current `BTreeMap` is a major scaling
cost. Synchronous single-input latency also contains several microseconds of
worker sleep/wake scheduling, but that is a separate optimization boundary.

The new representation targets history admission first. End-to-end
measurements remain in place so that improvements are not confused with
channel or scheduling costs.

## Considered Designs

### Selected: append history plus late-input tree

Store canonical in-order arrivals contiguously and store only out-of-order
arrivals in a `BTreeMap`. Merge the two ordered sources for replay and range
queries.

This gives the common path constant-time append behavior while retaining
logarithmic arbitrary middle insertion. The late tree stays small when the
workload behaves as expected.

### Rejected: fixed-size hot tail over a complete B-tree

A fixed vector containing the newest N inputs improves short hot-tail scans,
but after it fills every new input evicts an older input into the B-tree. It
therefore delays rather than removes the steady-state per-input tree insertion
cost. It also adds front movement or circular-buffer complexity.

### Rejected: segmented deque of sorted chunks

Bounded sorted chunks make front pruning, tail insertion, and middle insertion
predictable. They also introduce chunk search, split, merge, and capacity
policy that is unnecessary if late inputs are uncommon. The hybrid selected
design is simpler and gives a cheaper normal path.

### Rejected: time-bucket B-tree only

Grouping same-time inputs reduces the number of top-level tree entries, but
ordered inputs still interact with the tree and same-time ID insertion still
requires ordered insertion inside a bucket. It does not exploit the stronger
fact that most complete keys arrive in order.

## Architecture

### Global canonical identity

Input identity is global to one `Contime` instance and does not include time.
The router owns two logically separate structures:

```rust
struct CanonicalInputIndex<T> {
    retained_ids: HashSet<u128>,
    ids_by_retention_time: BTreeMap<T, Vec<u128>>,
}
```

`retained_ids` is the only identity lookup. If it contains an input ID, a new
submission with that ID is an accepted no-op even when its timestamp or
payload differs.

`ids_by_retention_time` is not an identity lookup. It records the canonical
time of the first accepted input solely so horizon advancement can remove the
corresponding IDs from `retained_ids`. IDs and retention records are added only
after admission succeeds. When the horizon removes an input, both records are
removed. A later submission of that formerly retained ID may then be accepted
again.

The existing global boundary remains authoritative for inputs routed to more
than one snapshot history. Per-snapshot storage must not redefine global
identity based on `(time, id)`.

### Per-snapshot history

Replace the public `inputs: BTreeMap<ContimeKey<Time>, Input>` implementation
with a focused internal abstraction:

```rust
struct HistoryInputs<T, I> {
    ordered: VecDeque<StoredInput<T, I>>,
    late: BTreeMap<ContimeKey<T>, I>,
}

struct StoredInput<T, I> {
    key: ContimeKey<T>,
    input: I,
}
```

`ordered` is an array-backed ring whose live entries are strictly increasing by
`ContimeKey`. It retains contiguous append behavior while allowing expired
front entries and their owned payloads to be dropped immediately without
shifting the remaining history. `late` contains inputs whose keys precede the
append tail at admission time.

`HistoryInputs` encapsulates all representation-specific operations needed by
history and checkpoint code:

- `len` and `is_empty` over live entries;
- `latest_key`;
- ordered and ranged iteration;
- first applicable event lookup for pending-history materialization;
- ordered batch admission;
- pruning before a key;
- conservative memory accounting.

No checkpoint or replay code reads either backing collection directly.
`LocalSnapshotHistory::inputs` remains available as a `HistoryInputs` value for
focused tests and benchmarks, with read-only length, emptiness, iteration, and
range methods. Code explicitly depending on the field's former `BTreeMap` type
must migrate to those representation-neutral methods. The crate is currently
API-unstable, and keeping a duplicate compatibility tree would erase the
optimization.

## Admission Algorithm

The router first performs global horizon validation and ID deduplication. An
input rejected as too old is not inserted into either canonical index. A
retained duplicate ID is a successful no-op and is not routed again.

For each routed per-snapshot batch:

1. Validate snapshot-lane compatibility for accepted event inputs.
2. Build each input's `ContimeKey { time, id }` once.
3. Detect the monotonic fast path while scanning the batch.
4. If every key is greater than the current ordered tail and the batch itself
   is strictly ordered, append the complete batch contiguously.
5. Otherwise, sort the new batch once by key and partition it:
   - keys greater than the ordered tail append to `ordered`;
   - earlier keys insert into `late`.
6. Return the earliest actually inserted time, inserted count, byte delta, and
   single-key optimization metadata required by checkpoint reconciliation.

The fallback sort does not alter semantics because replay order is canonical
key order rather than arrival order. Inputs accepted by the router are already
globally unique. Defensive debug assertions verify that neither collection
already contains an inserted ID/key.

When a history is empty, the first ordered batch establishes the append tail.
Once established, every late-tree key is less than the ordered tail that
existed when that key was inserted. Later appends can only increase that tail.

## Canonical Iteration and Replay

`HistoryInputs` provides a merge iterator over:

- the requested range of `ordered`;
- the corresponding `late.range(...)`.

At each step it yields the smaller `ContimeKey`. Equal keys are an invariant
violation because retained IDs are unique and a key may occur in only one
collection.

Replay consumes that iterator from left to right and groups adjacent inputs
with equal complete times into one raw `InputBatch`. Events at the same time
remain ordered by input ID. A reusable scratch vector holds references for the
current time bucket, avoiding a fresh allocation for every replay bucket.

The merged iterator replaces direct per-snapshot `BTreeMap` access for:

- checkpoint reconstruction;
- query-at-time replay;
- pending snapshot materialization;
- latest-key and range operations.

The worker's canonical input journal used by `inspect_inputs` remains a
separate structure in this pass. Its behavior is covered by regression tests,
but optimizing that journal is outside this history-storage change.

Checkpoint cadence continues to use the cumulative count of raw canonical
inputs. Late insertion invalidates and replays the same checkpoint suffix as
the current implementation.

## Horizon Pruning

Per-snapshot pruning uses the existing exclusive lower-bound key:
`ContimeKey { time: drop_time, id: u128::MIN }`.

- Pop ordered inputs from the front while their keys precede the boundary.
- Remove late-tree entries before the boundary.
- Subtract each removed input's conservative size from memory usage.
- Preserve the current replay-anchor checkpoint behavior.

Front pruning drops owned input payloads immediately and does not shift live
inputs. `VecDeque` may keep spare ring capacity under the standard allocator,
just as `Vec` and `BTreeMap` may retain allocation capacity, but no logically
pruned `Input` remains alive.

The router prunes `ids_by_retention_time` at the same horizon and removes those
IDs from `retained_ids`. Identity is therefore retained exactly as long as
canonical history. Pruned IDs are intentionally reusable if submitted again
within the then-current horizon.

## Invariants

- Retained identity is determined only by input ID, never by timestamp.
- A repeated retained ID is a no-op regardless of timestamp or payload.
- An input key exists in exactly one per-snapshot backing collection.
- The live ordered deque is strictly increasing by `(time, input_id)`.
- Merging the vector and late tree yields the complete canonical sequence.
- Same-time inputs form one replay batch ordered by input ID.
- Every globally retained ID has one retention-time entry.
- Horizon pruning forgets both canonical data and canonical identity.
- Checkpoints represent state after a complete time bucket.
- Public queries and inspection never expose the backing representation.

Debug builds assert these invariants at mutation boundaries. Tests compare the
hybrid representation against a simple canonical `BTreeMap` reference model.

## Memory and Failure Behavior

The design introduces no new recoverable public error. Duplicate IDs remain
successful no-ops. Existing memory-budget and before-horizon rejection behavior
is preserved. A lane mismatch or broken internal ordering remains an invariant
panic.

Memory accounting includes logical input sizes and conservative container/index
overhead using the crate's existing accounting conventions. Pruning drops each
owned input and releases its logical input charge. Retained allocator capacity
continues to follow the crate's existing approximate accounting policy. The
design does not attempt to make memory reservation transactional.

## Verification

### Correctness tests

- One and many ordered inputs.
- Same-time inputs in ascending, descending, and mixed ID arrival order.
- Late insertion at the front and middle.
- Batches containing both appendable and late inputs.
- Duplicate IDs with identical and different timestamps and payloads.
- Duplicate IDs routed to multiple snapshot histories.
- Horizon pruning from the ordered vector, late tree, and both together.
- Forgetting and reaccepting an ID after horizon removal.
- Replay and query ranges crossing the vector/tree boundary.
- Marker-only pending histories followed by materializing events.
- Checkpoint replay, checkpoint cadence, raw input counts, and replay anchors.
- Input inspection order and retention.
- Memory accounting while pruning ordered and late inputs.
- Property-style randomized comparison against a reference
  `BTreeMap<ContimeKey<Time>, Input>` plus an independent retained-ID set.

### Criterion benchmarks

Permanent benchmarks separate these layers:

1. Direct snapshot callback for 1, 100, and 1,000 inputs.
2. Raw `HistoryInputs` admission:
   - ordered unique timestamps;
   - ordered same timestamp;
   - slightly late insertion;
   - deeply late insertion;
   - mixed workloads at several late-input rates.
3. Merged range iteration and replay with empty, sparse, and dense late trees.
4. Horizon pruning with ordered-only, late-only, and mixed histories.
5. Persistent router `send` for same and separate snapshot lanes.
6. Persistent synchronous apply for same and separate snapshot lanes.

The existing diagnostic numbers are recorded as the before baseline. The
primary success signal for this pass is that ordered raw history admission
approaches linear contiguous-storage cost instead of scaling with one B-tree
operation per input. End-to-end results are reported separately; worker
sleep/wake and routing are not attributed to history storage.

## Implementation Boundary

This pass changes ConTime history storage, its canonical identity index, the
replay iterator, focused tests, Criterion benchmarks, and relevant README
performance documentation. It does not modify Timeless Runtime or add adaptive
worker spinning. Those optimizations should be considered only after the new
benchmarks identify the remaining costs.
