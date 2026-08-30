# ConTime Memory

`contime-memory` is the isolated owner of ConTime memory accounting. It has no
normal dependencies on the root crate or any sibling subcrate.

## Contracts

The crate separates three concerns:

- `ConservativeTrackedSize` measures the largest reasonable amount of memory
  currently retained by an underlying value. Implementations include inline
  storage and retained capacities, but exclude nested `TrackedArc` and
  `TrackedBox` handles because those account for themselves.
- `MemoryAccount<T>` measures a value around a mutation and produces an
  unsigned `MemoryChange`. `MeasuredAccount`, the default, measures before and
  after. `CachedAccount` stores the last size and measures only after mutation.
- `MemoryBudget` records allocation and pointer changes. The provided
  `AtomicMemoryBudget` is cloneable, lock-free, infallible during accounting,
  and uses `usize` counters.

All accounting records completed reality. A memory-growing operation is never
rolled back after it happened. Instead, `MemoryState` reports:

- `Ready` at or below the action ceiling;
- `ActionBlocked` above the action ceiling but at or below the hard limit;
- `HardLimitExceeded` above the hard limit.

The action ceiling is `hard_limit - concurrent_actions * action_buffer`. The
headroom is accounting policy rather than a physical allocation. An individual
increase larger than `action_buffer` increments `buffer_exceeded_count`, which
tells the runtime that its configured per-action safety margin was insufficient.

## Tracked ownership

`TrackedArc<T>` and `TrackedBox<T>` are each one machine pointer wide. Their
account and budget live beside `T` in the heap allocation.

`TrackedArc` charges the underlying allocation once and charges one pointer for
every ordinary `Clone`. Each handle drop releases its pointer; the final inner
drop releases the allocation. It exposes immutable access only.

`TrackedBox` owns one independently mutable allocation. Ordinary `Clone` deeply
clones `T`, creates a fresh account, and charges another allocation and pointer.
Mutation is available only through `update`, which measures the closure's memory
change and applies it to the budget. It deliberately has no `DerefMut` or
`into_inner` escape hatch.

Moving a tracked message through a channel changes no accounting. The message
remains charged until its tracked wrapper is dropped, irrespective of which
queue, sender, receiver, or process currently owns it.

## Verification

```sh
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo clippy --manifest-path crates/memory/Cargo.toml --all-targets -- -D warnings
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo bench --manifest-path crates/memory/Cargo.toml --bench lifecycle
```

Inline unit benchmarks are ignored tests. Run a unit by its filter, for example:

```sh
CARGO_TARGET_DIR=/private/tmp/contime-memory-target cargo test \
  --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_budget -- --ignored --nocapture --test-threads=1
```

The other filters are `benchmark_change`, `benchmark_measured_account`,
`benchmark_cached_account`, `benchmark_tracked_arc`, and
`benchmark_tracked_box`.

## Benchmark results

Measured on the current development machine in release mode with Criterion.
The median is the center estimate from the most recent run.

### Unit operations

| Operation | Batch | Median | Per operation | Throughput |
| --- | ---: | ---: | ---: | ---: |
| Calculate `MemoryChange` | 1,000 | 903.47 ns | 0.903 ns | 1.107B/s |
| Measured account, cheap sizing | 1,000 | 344.74 ns | 0.345 ns | 2.900B/s |
| Cached account, cheap sizing | 1,000 | 440.63 ns | 0.441 ns | 2.269B/s |
| Measured account, 1,000-value sizing | 1,000 | 1.3439 ms | 1.344 us | 744.1K/s |
| Cached account, 1,000-value sizing | 1,000 | 673.83 us | 673.83 ns | 1.484M/s |
| Reserve allocation bytes | 1,000 | 6.6776 us | 6.678 ns | 149.75M/s |
| Reserve pointer bytes | 1,000 | 6.6520 us | 6.652 ns | 150.33M/s |
| Resize increase | 1,000 | 6.6440 us | 6.644 ns | 150.51M/s |
| Resize decrease | 1,000 | 6.6799 us | 6.680 ns | 149.70M/s |
| Balanced pointer reserve/release | 1,000 pairs | 13.305 us | 13.305 ns/pair | 75.16M pairs/s |
| `TrackedArc::new` | 1 | 15.215 ns | 15.215 ns | 65.72M/s |
| `TrackedArc::clone` | 1 | 10.928 ns | 10.928 ns | 91.51M/s |
| Non-final Arc drop | 1 | 5.470 ns | 5.470 ns | 182.82M/s |
| Final Arc drop | 1 | 26.841 ns | 26.841 ns | 37.26M/s |
| Arc clone/drop | 1,000 pairs | 10.840 us | 10.840 ns/pair | 92.25M pairs/s |
| `TrackedBox::new` | 1 | 15.202 ns | 15.202 ns | 65.78M/s |
| Deep Box clone | 1 | 77.098 ns | 77.098 ns | 12.97M/s |
| Box drop | 1 | 51.743 ns | 51.743 ns | 19.33M/s |
| Measured Box update and drop | 1 | 47.900 ns | 47.900 ns | 20.88M/s |
| Cached Box update and drop | 1 | 55.516 ns | 55.516 ns | 18.01M/s |
| Grow Box vector by 1,000 bytes and drop | 1 | 70.573 ns | 70.573 ns | 14.17M/s |
| Deep Box clone/drop | 1,000 pairs | 74.904 us | 74.904 ns/pair | 13.35M pairs/s |

Cheap sizing favors the zero-sized measured account. Cached accounting becomes
useful when measuring the retained graph is expensive: the 1,000-value fixture
nearly halves measurement time by walking the graph once instead of twice.

### Integrated ownership flows

| Flow | Batch | Median | Per element | Throughput |
| --- | ---: | ---: | ---: | ---: |
| Create, send, receive, and drop tracked messages | 1,000 | 44.933 us | 44.933 ns | 22.256M/s |
| Measured fixed-size snapshot updates | 1,000 | 1.8296 us | 1.830 ns | 546.56M/s |
| Cached fixed-size snapshot updates | 1,000 | 2.1394 us | 2.139 ns | 467.42M/s |

The message benchmark prepares event allocations, the budget, and the channel
outside timing. It includes one Arc clone, tracked message allocation,
accounting, standard-channel send/receive, and message drop per element.

The snapshot benchmarks prepare and deeply clone their starting snapshots
outside timing. They include 1,000 `update` calls and amortize the final tracked
Box drop across the batch. The mutation changes existing elements without
changing retained capacity, intentionally isolating account/update overhead.

All unit measurements exclude API, router, worker, events, checkpoints, lanes,
and runtime orchestration. Criterion harness overhead and batched fixture setup
are excluded according to each benchmark's stated boundary.
