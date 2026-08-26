# Snapshot-Batched Apply Pipeline Implementation Report

**Date:** 2026-08-26  
**Implementation base:** `b4dc3a9` (`bench: measure apply pipeline boundaries`)  
**Verified implementation and documentation:** `ed71b80` (`fix: reserve complete checkpoint cadence`)  
**Branch:** `codex/router-api-boundary`

## Result

ConTime now has one explicit, typed apply path:

```text
API inputs -> snapshot batches -> worker messages -> snapshot histories
```

The API performs route visitation and stable per-snapshot grouping once. The
router hashes only prepared snapshot IDs and sends one complete message to each
affected worker. A worker reserves its whole message, then applies each prepared
batch directly to its snapshot history. Duplicate identity and history-horizon
decisions live in each snapshot history.

Retained event memory and apply-time allocation are separate contracts:

- `Input::conservative_size` is the retained input footprint.
- `Event::conservative_allocation_size` is extra snapshot-state allocation
  caused by applying the event.
- `Snapshot::conservative_size` is checkpoint state.
- `#[contime_event(..., allocation_bytes = ...)]` supplies the derived event
  allocation estimate; omitted values default to zero.

The old retained-input inspection/journal path and worker admission subsystem
are absent.

## RED/GREEN evidence

### Task 1 — identity and horizon admission in snapshot history

RED:

- duplicate identity remained worker-owned/key-scoped;
- pruning did not forget history-local identity;
- routed history apply APIs did not exist.

GREEN:

- `cargo test history::inputs -- --nocapture`
- `cargo test history::storage -- --nocapture`
- `cargo test --test edge -- --nocapture`
- `cargo test --test history_input_count -- --nocapture`

Commit: `9f08d80`.

### Task 2 — ordered snapshot batches at the API boundary

RED: `cargo test --test snapshot_batching -- --nocapture` failed to compile
because the grouping adapter and production batch type did not exist.

GREEN: snapshot grouping preserved first-snapshot order, per-snapshot input
order, dynamic marker routes, and empty-route discard behavior.

Commit: `f251b13`.

### Task 3 — remove retained-input inspection and worker journal

RED: the public-boundary test found the inspection types, methods, dispatch
path, and journal storage.

GREEN: the symbols were removed; a trybuild fixture now proves the removed API
does not compile, while input, fragment, and memory behavior stayed green.

Commit: `6481462`.

### Task 4 — shared memory tracking

RED: memory tracker unit tests failed to compile because no shared tracker
existed.

GREEN: advisory checks, atomic reservations, release, positive reconciliation,
and negative deltas passed, with one tracker shared by API, router, and workers.

Commit: `fe9cddc`.

### Task 5 — direct snapshot-batch routing and worker application

RED:

- direct router/worker tests failed to compile for missing snapshot-batch
  adapters;
- API precheck tests failed to compile for missing `ContimeError::MemoryFull`;
- full testing then caught underestimated replay/checkpoint growth;
- the derived vector-growth fixture demonstrated why retained event bytes and
  apply allocation cannot share one estimate.

GREEN:

- complete snapshot batches partition and dispatch without reopening inputs;
- worker messages reserve atomically and reject without partial mutation inside
  that worker message;
- synchronous multi-worker rejections are request-scoped and deduplicated;
- replay checkpoint space is reserved conservatively;
- event payload, apply allocation, identity, and clean checkpoint bytes are
  each accounted once;
- derived and manual allocating events declare apply allocation separately.

Commit: `0372a5b`.

### Independent review corrections

An independent review of `b4dc3a9..0ecbbf0` found checkpoint-allocation
amplification, incomplete checkpoint-key accounting, stale-only unseen history
creation, and a source-compatibility gap in `InputLanes`.

RED reproduced the critical case exactly: 1,000 distinct-time events grew
history by 85,360 bytes against an 82,036-byte reservation, panicking the
worker and returning `ResponseDisconnected`.

GREEN now proves:

- fresh batches reserve apply allocation across every possible retained
  checkpoint copy;
- late events reserve allocation across existing replayed checkpoints;
- checkpoint accounting includes `ContimeKey<Time>`, snapshot state, and the
  history input count;
- stale-only batches for unseen IDs are rejected before inserting an empty
  history;
- manual `InputLanes` implementations retain a zero-allocation default; and
- one event routed to two workers reports one deduplicated rejection when only
  one route applies.

Commit: `ebb3d6c`.

A focused re-review then found that zero-allocation inline events still create
full cadence checkpoints. RED reproduced 64,720 bytes of actual growth against
a 64,072-byte reservation for 1,000 such events. The final estimate now
multiplies the complete checkpoint footprint—not only apply allocation—across
every possible cadence copy. The zero-allocation regression is GREEN.

Commit: `ed71b80`.

## Matched 1,000-event benchmark

Command:

```bash
cargo bench --bench apply_boundaries -- apply_1000_events_one_snapshot --sample-size 30
```

Environment: Apple M3 Pro, macOS 26.3.1 (25D771280a), rustc 1.90.0, optimized
benchmark profile. Intervals are Criterion `[low estimate high]`.

| Entry boundary | 1,000-event interval | Point estimate/event |
| --- | ---: | ---: |
| Public API | `[64.473 µs 64.929 µs 65.531 µs]` | `64.929 ns` |
| Router | `[55.431 µs 55.676 µs 56.007 µs]` | `55.676 ns` |
| Worker | `[55.301 µs 55.624 µs 56.084 µs]` | `55.624 ns` |
| Snapshot history | `[41.963 µs 42.083 µs 42.164 µs]` | `42.083 ns` |

Adjacent point-estimate residuals:

- API minus router: `9.253 µs`; confidence intervals do not overlap.
- Router minus worker: `52 ns`; confidence intervals overlap, so this cost is
  not separable from scheduling noise.
- Worker minus history: `13.541 µs`; confidence intervals do not overlap.

The largest remaining outer-layer residual is worker entry to direct history
entry. Direct history work remains the largest component of the complete API
round trip.

## Verification

The following exact gates passed on the implementation/documentation state:

```bash
cargo bench --bench apply_boundaries --no-run
cargo bench --bench apply --no-run
cargo bench --bench router --no-run
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --doc
git diff --check
```

`cargo test --all-targets` passed 42 library unit tests, 98 integration test
functions (including the trybuild boundary suite), all Criterion smoke cases,
and the example target. The doctest passed separately.

The removed-symbol search is empty in production, benchmarks, README, and
ordinary tests. The only repository occurrences are the intentional
`tests/ui/removed_input_inspection.rs` compile-fail input and its expected
stderr.

## Provisional memory limitation

The API precheck is advisory. Concurrent calls can both pass before either
worker reserves memory. Workers reserve independently, so an event routed to
several workers—or a request spanning snapshots on several workers—can be
applied by one worker and rejected by another. Synchronous `apply` reports the
affected event IDs and reason codes; asynchronous `send` is best effort after
enqueue. Conservative estimates can over-reject. Cross-worker transactional
reservation and rollback remain deferred.
