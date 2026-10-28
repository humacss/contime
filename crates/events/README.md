# contime-events

`contime-events` is an isolated, passive event-history store. It owns no
threads, channels, snapshots, checkpoints, worker scheduling, memory limits,
or ConTime orchestration.

Core adapts this workspace member into each snapshot history and connects it
to checkpoint storage. The event store itself remains independent of those
crates and knows nothing about their execution topology.

## API guide

Implement [`Event`](https://github.com/humacss/contime/blob/main/crates/events/src/types.rs) for the owned event value or shared handle you
want to store. Its time and ID must stay stable while retained. Create an
[`EventHistory`](https://github.com/humacss/contime/blob/main/crates/events/src/history.rs) with `new`, `with_capacity`, or `with_horizon`,
then call `insert`. Only `Insert::Inserted` changes replay dirtiness; events before
the horizon are rejected before duplicate detection.

Use `iter` for a borrowed canonical history, `iter_from_dirty` for whole-timestamp
replay, or `clone_between(from, to)` for owned results over `[from, to)`. Empty
and reversed query ranges return an empty vector. Queries that start before the
horizon return only retained events and do not report missing older history.
`iter_after` excludes one exact `(time, event_id)` key, rather than the entire
timestamp. `prune_before` advances the horizon monotonically and forgets removed
IDs. These operations never apply events to consumer state.

Generate the public Rust API reference from the workspace root:

```sh
cargo doc -p contime-events --no-deps --open
```

## Responsibilities

- Accept any event ownership representation selected by the consumer.
- Deduplicate retained events by event ID, independently of timestamp.
- Append canonically ordered events to a `VecDeque`.
- Store out-of-order events in a `BTreeMap`.
- Iterate both stores as one canonical `(time, event ID)` history.
- Track the earliest timestamp from which checkpoint replay must resume.
- Iterate from that dirty timestamp while retaining its complete timestamp
  bucket.
- Expose basic length, latest-key, and storage-count inspection.

The store also owns a monotonic history horizon. Events strictly before it are
rejected before duplicate detection, while events exactly at it remain valid.
`prune_before` removes obsolete late-tree and ordered-prefix entries and
forgets their IDs, allowing those IDs to be reused by retained-time events.
Checkpoint materialization belongs to the snapshot store.

## Public boundary

Consumers implement the local `Event` trait by providing an ordered time and a
`u128` event ID. The time type's `Default` value is its zero timestamp.
`EventHistory<E>` stores `E` directly as its consumer-selected ownership type
and is the per-snapshot state object. It does not store a snapshot
ID; the worker owns the mapping from snapshot IDs to histories.

`EventHistory::clone_between` returns owned clones in canonical
`(time, event_id)` order over the half-open interval `[from, to)`. The merged
iterator remains a local borrow; every selected value is cloned before the
result can cross a crate or thread boundary. Pointer-backed event types clone
their handles rather than their payloads.

An empty history starts dirty at zero. Its first unique event sets the dirty
timestamp to that event's time, and subsequent unique events may move the
timestamp earlier. Duplicate IDs do not change it. `mark_replayed` moves the
boundary to the latest retained timestamp, conservatively preserving the last
same-timestamp bucket for the next replay. The first unique event accepted
after that acknowledgement starts a new dirty interval at its own timestamp;
this lets downstream consumers distinguish later changes from the conservative
clean-history boundary.

## Benchmark snapshot

Horizon-pruning results recorded on 2026-09-01:

| Pruned events | Storage | Total | Per event |
| ---: | --- | ---: | ---: |
| 1 | Late tree | 123.7 ns | 123.7 ns |
| 100 | Late tree | 3.31 us | 33.1 ns |
| 1,000 | Late tree | 28.1 us | 28.1 ns |
| 1,000 | Ordered deque prefix | 22.55 us | 22.55 ns |

Local release-mode Criterion results on 2026-08-29:

| Insertion workload | 1,000 events | ns/event | Events/s |
| --- | ---: | ---: | ---: |
| Ordered append | 7.045 us | 7.04 | 142.0 million |
| Late tree insertion | 43.082 us | 43.08 | 23.2 million |
| Duplicate rejection | 5.300 us | 5.30 | 188.7 million |

Insertion fixtures and final history destruction are outside the timed
section. The measurement includes event-ID deduplication, event-key creation,
dirty-time maintenance, and the selected storage operation. Duplicate
rejection includes dropping the rejected incoming `Arc`.

| Iteration workload | 1,000 events | ns/event | Events/s |
| --- | ---: | ---: | ---: |
| Full history, 0% late | 1.375 us | 1.38 | 727.3 million |
| Full history, 10% late | 1.509 us | 1.51 | 662.7 million |
| Full history, 50% late | 2.068 us | 2.07 | 483.5 million |
| Dirty range over all ordered events | 2.450 us | 2.45 | 408.2 million |

The dirty-range case includes locating the deque boundary, constructing both
range cursors, and visiting all 1,000 events. Direct traversal of the same
ordered deque measured 349.84 ns, providing a lower-bound reference for the
merged iterator abstraction.

## Integration benchmark snapshot

The public-API integration benchmark measures batches of 1,000 events behind
a benchmark-local `Arc` wrapper. Production event storage does not prescribe
that wrapper. Event construction, `Arc` allocation, fixture preparation,
and fixture destruction are outside the timed section. Each result includes
moving every wrapper from the prepared input vector through `EventHistory::insert`;
duplicate insertion also includes dropping the rejected incoming wrapper.

Local release-mode Criterion results on 2026-08-29:

| Event size | Ordered 1,000 | Ordered ns/event | Ordered events/s | Late 1,000 | Late ns/event | Late events/s | Duplicate 1,000 | Duplicate ns/event | Duplicate events/s |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 32 bytes | 7.685 us | 7.68 | 130.1 million | 43.882 us | 43.88 | 22.8 million | 5.305 us | 5.30 | 188.5 million |
| 64 bytes | 7.579 us | 7.58 | 131.9 million | 43.536 us | 43.54 | 23.0 million | 5.440 us | 5.44 | 183.8 million |
| 208 bytes | 8.500 us | 8.50 | 117.6 million | 49.300 us | 49.30 | 20.3 million | 6.181 us | 6.18 | 161.8 million |
| 1,008 bytes | 7.714 us | 7.71 | 129.6 million | 46.965 us | 46.97 | 21.3 million | 5.842 us | 5.84 | 171.2 million |

The payload remains behind an `Arc` and is not copied during insertion, so the
results do not grow proportionally with event size. Ordered insertion is
primarily retained-ID hashing plus deque append; late insertion additionally
pays for ordered B-tree insertion.

The representative 208-byte iteration cases produced:

| Iteration workload | 1,000 events | ns/event | Events/s |
| --- | ---: | ---: | ---: |
| Full history | 1.364 us | 1.36 | 733.0 million |
| Dirty range over all events | 2.449 us | 2.45 | 408.3 million |

Event queries clone the selected `Arc`-backed handles into an owned vector.
Local release-mode Criterion results on 2026-09-01:

| Late storage | 0 results | 10 results | 100 results | 1,000 results | ns/result at 1,000 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0% | 10.866 ns | 701.56 ns | 2.7176 us | 20.351 us | 20.35 |
| 10% | 8.6813 ns | 796.72 ns | 5.6155 us | 35.295 us | 35.30 |
| 50% | 11.969 ns | 734.30 ns | 3.5488 us | 29.153 us | 29.15 |

The query measurements include locating the lower bound, merging ordered and
late stores, allocating the result vector, cloning each selected shared handle,
and dropping the returned vector. Event payloads are not cloned.

Run the integration suite with:

```bash
cargo bench --manifest-path crates/events/dev/Cargo.toml --bench events
```

## Full benchmark run — 2026-10-28

Measured on Apple M3 Pro (11 logical CPUs), Rust 1.90.0, optimized Cargo bench
profile, with unpinned threads and normal desktop background activity. All groups
ran sequentially after compilation. Existing warm-up, sample-count and measurement
settings were retained; no shortened timing overrides were used. These are local
measurements, not production-capacity guarantees. The workspace lockfile uses
Criterion 0.5.1, Crossbeam channel 0.5.15 and Crossbeam utils 0.8.21.
Earlier dated results above
remain historical records and may use different fixtures or settings.

Tables report the **median time per benchmark iteration**, with its bootstrap
95% confidence interval. Rates are derived from that median only where the
benchmark declares a throughput unit; that unit may be source events, target
applications, snapshots, bytes, or bookkeeping calls. It is not automatically
ConTime's event-processing capacity. Thread-startup and allocation boundaries
remain those defined in each benchmark source.

Feature variants were measured in separate sequential runs, not interleaved
paired trials; changing background load can affect their differences.

Feature sets: `all-features` is the workspace feature union, including optional
worker counters; `default` disables optional counters; `status-no-counters`
enables Monitor's `runtime` feature without counters. Each table states the
actual sample count. Inline tests may contain multiple Criterion cases.

### `events` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `events/insertion/duplicate/1008_bytes` | 4.767 µs | 4.752 µs–4.778 µs | 100 | 209.785 M elements/s |
| `events/insertion/duplicate/208_bytes` | 5.999 µs | 5.957 µs–6.024 µs | 100 | 166.685 M elements/s |
| `events/insertion/duplicate/32_bytes` | 4.536 µs | 4.525 µs–4.563 µs | 100 | 220.463 M elements/s |
| `events/insertion/duplicate/64_bytes` | 4.556 µs | 4.532 µs–4.639 µs | 100 | 219.476 M elements/s |
| `events/insertion/late/1008_bytes` | 37.638 µs | 37.439 µs–37.887 µs | 100 | 26.569 M elements/s |
| `events/insertion/late/208_bytes` | 42.416 µs | 42.307 µs–42.500 µs | 100 | 23.576 M elements/s |
| `events/insertion/late/32_bytes` | 34.065 µs | 34.007 µs–34.140 µs | 100 | 29.355 M elements/s |
| `events/insertion/late/64_bytes` | 34.285 µs | 34.189 µs–34.356 µs | 100 | 29.167 M elements/s |
| `events/insertion/ordered/1008_bytes` | 6.660 µs | 6.595 µs–6.716 µs | 100 | 150.154 M elements/s |
| `events/insertion/ordered/208_bytes` | 7.811 µs | 7.762 µs–7.901 µs | 100 | 128.026 M elements/s |
| `events/insertion/ordered/32_bytes` | 5.993 µs | 5.967 µs–6.016 µs | 100 | 166.853 M elements/s |
| `events/insertion/ordered/64_bytes` | 6.113 µs | 6.103 µs–6.131 µs | 100 | 163.577 M elements/s |
| `events/iteration/208_byte_events/from_dirty` | 1.922 µs | 1.921 µs–1.923 µs | 100 | 520.355 M elements/s |
| `events/iteration/208_byte_events/full_history` | 1.077 µs | 1.076 µs–1.079 µs | 100 | 928.621 M elements/s |
| `events/pruning/late_tree/1` | 94.390 ns | 94.014 ns–94.747 ns | 100 | 10.594 M elements/s |
| `events/pruning/late_tree/100` | 2.614 µs | 2.605 µs–2.631 µs | 100 | 38.262 M elements/s |
| `events/pruning/late_tree/1000` | 22.149 µs | 22.091 µs–22.254 µs | 100 | 45.149 M elements/s |
| `events/pruning/ordered_deque/1000_events` | 17.887 µs | 17.866 µs–17.912 µs | 100 | 55.907 M elements/s |
| `events/query/0_percent_late/0_results` | 1.940 ns | 1.939 ns–1.942 ns | 100 | — |
| `events/query/0_percent_late/1000_results` | 4.501 µs | 4.480 µs–4.511 µs | 100 | — |
| `events/query/0_percent_late/100_results` | 603.019 ns | 602.512 ns–603.826 ns | 100 | — |
| `events/query/0_percent_late/10_results` | 168.796 ns | 168.463 ns–169.070 ns | 100 | — |
| `events/query/10_percent_late/0_results` | 1.964 ns | 1.951 ns–1.989 ns | 100 | — |
| `events/query/10_percent_late/1000_results` | 5.222 µs | 5.214 µs–5.234 µs | 100 | — |
| `events/query/10_percent_late/100_results` | 751.108 ns | 748.231 ns–753.584 ns | 100 | — |
| `events/query/10_percent_late/10_results` | 172.258 ns | 171.777 ns–172.760 ns | 100 | — |
| `events/query/50_percent_late/0_results` | 1.976 ns | 1.975 ns–1.978 ns | 100 | — |
| `events/query/50_percent_late/1000_results` | 5.131 µs | 5.118 µs–5.147 µs | 100 | — |
| `events/query/50_percent_late/100_results` | 734.462 ns | 730.754 ns–737.181 ns | 100 | — |
| `events/query/50_percent_late/10_results` | 183.969 ns | 181.640 ns–187.359 ns | 100 | — |

### `contime_events` · inline · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `events/advance/1000_ordered` | 8.679 µs | 8.655 µs–8.706 µs | 100 | — |
| `events/history/1000_metadata_reads` | 934.545 ns | 929.679 ns–943.555 ns | 100 | — |
| `events/history/create_with_capacity_1000` | 645.233 ns | 618.433 ns–685.820 ns | 100 | — |
| `events/insert/1000_duplicate` | 3.608 µs | 3.587 µs–3.630 µs | 100 | — |
| `events/insert/1000_late` | 33.963 µs | 33.835 µs–34.219 µs | 100 | — |
| `events/insert/1000_ordered` | 5.556 µs | 5.498 µs–5.619 µs | 100 | — |
| `events/insert/1000_ordered_reserved_in_timed_path` | 6.328 µs | 6.274 µs–6.364 µs | 100 | — |
| `events/insert/1000_ordered_unreserved` | 19.533 µs | 19.501 µs–19.594 µs | 100 | — |
| `events/iteration/1000_events/0_percent_late` | 1.191 µs | 1.189 µs–1.193 µs | 100 | — |
| `events/iteration/1000_events/10_percent_late` | 1.267 µs | 1.237 µs–1.274 µs | 100 | — |
| `events/iteration/1000_events/50_percent_late` | 1.383 µs | 1.379 µs–1.388 µs | 100 | — |
| `events/iteration/1000_events/from_dirty` | 2.127 µs | 2.124 µs–2.132 µs | 100 | — |
| `events/iteration/1000_events/ordered_storage_direct` | 283.706 ns | 282.128 ns–296.744 ns | 100 | — |
| `events/query/clone_between/1000_events` | 3.833 µs | 3.822 µs–3.840 µs | 100 | — |
