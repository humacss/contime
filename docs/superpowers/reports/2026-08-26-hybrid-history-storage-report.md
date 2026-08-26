# Hybrid History Storage Implementation Report

Date: 2026-08-26  
Repository: `humacss/contime`  
Branch: `codex/hybrid-history-storage`

## Outcome

ConTime snapshot histories now use a hybrid representation:

- canonical in-order arrivals append to a `VecDeque`;
- arrivals before the append tail enter a `BTreeMap`;
- replay merges both sorted sources by `(time, input ID)`;
- horizon advancement prunes both stores and preserves a replay anchor;
- retained input identity is global and timestamp-independent until pruning;
- an ID is forgotten when its retained input crosses the history horizon.

Timeless Runtime was not modified. The benchmark work deliberately separates
direct snapshot callbacks, snapshot history, router/worker send, synchronous
ConTime apply, and the unmeasured outer Runtime boundary.

## Commits

| Work | Commit |
| --- | --- |
| Prerequisite checkpoint reproduced in the isolated worktree | `45c1f7e` |
| Task 1: trustworthy apply-cost benchmarks | `fc62520` |
| Task 2: hybrid `HistoryInputs` store | `f41baa3` |
| Task 3: snapshot replay and pruning integration | `1378d6f` |
| Task 4: identity/retention separation | `993452a` |
| Task 5: randomized reference equivalence | `4d0ab54` |
| Task 6: late/replay/pruning benchmark matrix | `520e386` |
| Task 7: README architecture and performance record | `aa408f2` |
| Strict-Clippy private return-type cleanup | `6fa3186` |

The design and execution plan were committed earlier as `cb260d7` and
`7211121`. The original dirty checkout was not modified; implementation ran in
`.worktrees/hybrid-history-storage`.

## RED/GREEN evidence

### Task 1 — benchmark validity

RED: the first persistent send/apply measurements were approximately 130 ns
because Criterion's batched setup advanced the shared horizon before timed
iterations. Timed inputs were stale rejected no-ops, so the harness was not
measuring apply work.

GREEN: the harness now uses `iter_custom`, performs synchronization and fixture
creation outside each timed interval, and supplies unique retained inputs. The
resulting direct-history and persistent matrices scale with their real work.

### Task 2 — hybrid store

RED: focused tests initially failed to compile because `HistoryInputs` and its
ordered/late APIs did not exist.

GREEN: seven unit tests passed for append admission, middle late admission,
same-time ID ordering, exact-key deduplication, bounded ranges, latest-key
selection, and pruning both stores.

### Task 3 — history integration

RED: focused integration cases exercised behavior unavailable through the old
single-store path: ordered-plus-late merged replay, a lower ID arriving at the
same time, and horizon pruning across both stores into the replay anchor.

GREEN: the history suite passed 27/27, `history_input_count` passed 8/8, and
`horizon_compaction` passed 1/1 after admission, replay, checkpoint, and pruning
were routed through `HistoryInputs`.

### Task 4 — global identity

The duplicate-ID and horizon-reuse tests were characterization tests and were
green before the internal refactor. They remained green after replacing the
timestamp-keyed identity index with a retained-ID set plus retention-time
buckets. This deliberately preserved behavior while making the contract
explicit: time does not participate in identity, but it does control forgetting.

### Task 5 — reference equivalence

RED: the randomized integration test failed to compile because the
representation-neutral `prune_before_time` and `latest_entry_key` observations
were missing.

GREEN: 32 deterministic seeds × 1,000 operations matched a canonical
`BTreeMap` and retained-ID set after every ordered insertion, late insertion,
duplicate submission, and prune. `cargo test --tests -q` passed all integration
targets.

### Task 6 — focused workloads

GREEN: the apply benchmark compiled and every Criterion smoke fixture passed.
Release measurements completed for 0/1/10/50% late admission, 0/10/50% merged
replay density, ordered/late/mixed pruning, persistent send, and synchronous
apply.

### Task 7 — final gate

RED: strict Clippy found `clippy::type_complexity` on the private router result
tuple introduced by the prerequisite checkpoint.

GREEN: a private `RoutedInputsResult` alias removed the warning without changing
behavior. Strict all-target/all-feature Clippy then passed.

## Criterion evidence

Environment: Apple M3 Pro, macOS 26.3.1 (25D771280a), rustc 1.90.0,
optimized benchmark profile, Criterion sample size 20. Values are exact
`[low estimate high]` intervals.

### Direct boundaries

| Boundary | Inputs | Before | After |
| --- | ---: | ---: | ---: |
| Callback | 1 | `[38.058 ns 38.940 ns 40.238 ns]` | `[40.840 ns 40.933 ns 41.017 ns]` |
| Callback | 100 | `[115.35 ns 116.21 ns 117.57 ns]` | `[115.53 ns 117.46 ns 120.68 ns]` |
| Callback | 1,000 | `[2.4934 µs 3.4223 µs 4.3563 µs]` | `[1.2837 µs 1.2869 µs 1.2922 µs]` |
| Snapshot history | 1 | `[223.34 ns 246.78 ns 278.95 ns]` | `[176.37 ns 177.35 ns 178.02 ns]` |
| Snapshot history | 100 | `[3.9931 µs 4.0529 µs 4.1687 µs]` | `[4.0462 µs 4.0919 µs 4.1213 µs]` |
| Snapshot history | 1,000 | `[47.938 µs 49.951 µs 53.512 µs]` | `[46.908 µs 47.320 µs 47.651 µs]` |

The callback implementation did not change. Its 1,000-input difference is not
attributed to hybrid history. The 100-input direct-history intervals overlap.

### Persistent boundaries

| Boundary | Shape | Inputs | Before | After |
| --- | --- | ---: | ---: | ---: |
| `send` | same snapshot | 1 | `[730.96 ns 828.89 ns 956.92 ns]` | `[657.51 ns 676.39 ns 694.33 ns]` |
| `send` | same snapshot | 100 | `[18.343 µs 18.468 µs 18.614 µs]` | `[16.146 µs 16.234 µs 16.378 µs]` |
| `send` | same snapshot | 1,000 | `[180.49 µs 181.02 µs 181.77 µs]` | `[142.32 µs 142.62 µs 142.94 µs]` |
| `send` | separate snapshots | 1 | `[766.91 ns 938.61 ns 1.0867 µs]` | `[619.53 ns 624.41 ns 632.50 ns]` |
| `send` | separate snapshots | 100 | `[19.947 µs 21.736 µs 23.592 µs]` | `[16.205 µs 16.428 µs 16.829 µs]` |
| `send` | separate snapshots | 1,000 | `[182.01 µs 193.87 µs 207.18 µs]` | `[141.73 µs 142.13 µs 142.55 µs]` |
| synchronous `apply` | same snapshot | 1 | `[2.9311 µs 3.8221 µs 4.8290 µs]` | `[2.7049 µs 2.7469 µs 2.7975 µs]` |
| synchronous `apply` | same snapshot | 100 | `[43.474 µs 49.559 µs 56.384 µs]` | `[37.688 µs 37.795 µs 37.936 µs]` |
| synchronous `apply` | same snapshot | 1,000 | `[317.77 µs 348.64 µs 402.39 µs]` | `[249.55 µs 249.86 µs 250.19 µs]` |
| synchronous `apply` | separate snapshots | 1 | `[3.6545 µs 3.9664 µs 4.2639 µs]` | `[2.7662 µs 2.9599 µs 3.1615 µs]` |
| synchronous `apply` | separate snapshots | 100 | `[58.911 µs 61.139 µs 62.896 µs]` | `[55.813 µs 56.236 µs 56.904 µs]` |
| synchronous `apply` | separate snapshots | 1,000 | `[451.22 µs 457.48 µs 469.68 µs]` | `[428.98 µs 429.62 µs 430.72 µs]` |

### Hybrid workloads

| Workload | Shape | Interval |
| --- | --- | ---: |
| Insert/reconcile 1,000 | 0% late | `[43.243 µs 43.345 µs 43.408 µs]` |
| Insert/reconcile 1,000 | 1% late | `[57.002 µs 57.115 µs 57.203 µs]` |
| Insert/reconcile 1,000 | 10% late | `[73.544 µs 73.833 µs 74.089 µs]` |
| Insert/reconcile 1,000 | 50% late | `[80.871 µs 82.473 µs 85.870 µs]` |
| Replay 1,000 | 0% late | `[6.0963 µs 6.1214 µs 6.1449 µs]` |
| Replay 1,000 | 10% late | `[5.9662 µs 5.9956 µs 6.0129 µs]` |
| Replay 1,000 | 50% late | `[3.8389 µs 3.9031 µs 3.9426 µs]` |
| Prune at boundary 500 | ordered only | `[4.6183 µs 4.6750 µs 4.7083 µs]` |
| Prune at boundary 500 | late only | `[12.625 µs 12.791 µs 12.882 µs]` |
| Prune at boundary 500 | mixed | `[4.5880 µs 4.6614 µs 4.7208 µs]` |

## Final verification

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --all-targets`: passed — 27 unit tests and 83 top-level
  integration tests, plus all Criterion smoke targets and the example target.
- `cargo test --doc`: passed — 1 doctest.
- `cargo bench --bench apply --no-run`: passed; optimized benchmark executable
  built successfully.
- `git diff --check`: passed before the report commit.

## Remaining costs and follow-up boundary

Direct merged replay is about 3.9–6.1 µs for 1,000 inputs, while a synchronous
single-input apply remains about 2.7–3.0 µs. That gap is not explained by
history replay alone: routing, worker dispatch, wake-up, reply-channel
round-trip, and synchronization remain in the ConTime boundary. Outer Timeless
Runtime orchestration remains a separate future investigation.

