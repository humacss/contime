# contime-router

`contime-router` receives complete input batches, deterministically maps each
snapshot route to a worker, and sends one final batch per affected worker.

The crate is currently isolated: the root `contime` crate does not use it, and
it is not a workspace member. It has no dependency on `contime` or
`contime-api`.

## Responsibilities

- Receive router-local input batches.
- Hash snapshot IDs from a caller-provided seed.
- Flatten directly into final worker vectors.
- Send one batch per affected worker.

Every input event enters the router in `Arc`. A single snapshot route moves the
existing Arc without cloning it; each additional snapshot route clones only the
pointer. Workers receive exact `{ snapshot_id, Arc<Event> }` records and never
revisit or rehash an event's snapshot IDs. The underlying event implements
`RoutableInput` and does not need to implement `Clone`.

## Exclusions

The crate owns no threads, workers, memory accounting, queries, time
advancement, rejection semantics, response waiting, or recovery orchestration.

## Verification

Run the inline unit tests from the ConTime repository root:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib
```

Run the inline Criterion benchmark separately in release mode:

```bash
cargo test --manifest-path crates/router/Cargo.toml --release --lib benchmark_route -- --ignored --nocapture
cargo test --manifest-path crates/router/Cargo.toml --release --lib benchmark_hash -- --ignored --nocapture
```

Run the sustained integration benchmark:

```bash
cargo bench --manifest-path crates/router/Cargo.toml --bench router
```

Generate its CPU flame graph with optimized code and full symbols:

```bash
CARGO_PROFILE_BENCH_DEBUG=2 cargo bench --manifest-path crates/router/Cargo.toml --bench router -- 64_byte_events/8_workers/1_route --profile-time 10
```

Criterion writes the graph to
`target/criterion/router_100_batches_1000_inputs_64_byte_events_8_workers/1_route/profile/flamegraph.svg`
inside this crate.

## Benchmarks

The following measurement was recorded locally in release mode on 2026-08-28:

| Benchmark | Time | Amortized per item |
| --- | ---: | ---: |
| `router/1000_inputs/8_workers` | 4.1140–4.1542 µs | approximately 4.14 ns |
| `hash/1000_snapshot_ids/8_workers` | 924.89–927.89 ns | approximately 0.926 ns |

Using the benchmark medians, hashing accounts for approximately **22.4%** of
the complete routing time: 926.17 ns of 4.1373 µs. The remaining routing work
accounts for approximately 3.211 µs, or 77.6%.

The unit fixture uses seed `7`, one batch containing 1,000 Arc-owned,
single-destination inputs, eight worker outputs, and a real Crossbeam
completion sender. Input construction and all channel construction happen
outside the timed routine. Worker processing is not part of this crate and is
also excluded.

The measurement includes:

- receiving the already-enqueued input batch;
- deriving the private AHash state from the seed;
- visiting and hashing all 1,000 snapshot IDs;
- allocating the worker-slot and affected-worker vectors;
- moving routed inputs directly into final worker batches;
- cloning the completion sender once per additional affected worker;
- sending one batch through each affected worker's real Crossbeam channel;
- observing normal shutdown when the input channel disconnects.

The isolated hash measurement includes creating the seeded AHash state once,
hashing 1,000 `u128` snapshot IDs, reducing each hash to one of eight worker
indexes, and accumulating the results so the optimizer must retain the work.
The snapshot-ID vector is constructed outside the timed routine.

### Sustained routing

The integration benchmark prepares 100 complete batches of 1,000 inputs before
timing begins. The following historical matrix compared direct ownership with
a benchmark-local consumer `Arc` wrapper and provided the evidence for making
the public boundary Arc-only. The owned cases are no longer executable through
the current router API:

| Event bytes | Routes per event | Owned total | Owned per route | Shared total | Shared per route |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 32 | 1 | 310.98 µs | 3.110 ns | 308.22 µs | 3.082 ns |
| 64 | 1 | 985.67 µs | 9.857 ns | 699.83 µs | 6.998 ns |
| 64 | 2 | 1.9858 ms | 9.929 ns | 1.3813 ms | 6.907 ns |
| 64 | 3 | 3.1544 ms | 10.515 ns | 2.0980 ms | 6.993 ns |
| 208 | 1 | 2.3490 ms | 23.490 ns | 869.04 µs | 8.690 ns |
| 208 | 2 | 4.7359 ms | 23.680 ns | 1.4820 ms | 7.410 ns |
| 208 | 3 | 7.5927 ms | 25.309 ns | 2.2519 ms | 7.506 ns |
| 1,008 | 1 | 8.3363 ms | 83.363 ns | 739.31 µs | 7.393 ns |
| 1,008 | 2 | 15.626 ms | 78.130 ns | 1.3966 ms | 6.983 ns |
| 1,008 | 3 | 26.241 ms | 87.470 ns | 2.1705 ms | 7.235 ns |

`SnapshotIds` has one-, two-, and three-ID variants, so snapshot-ID count is
the route count without a separate field. The payload lengths are 0, 144, and
944 bytes; together with snapshot routing data and alignment they produce
events of exactly 64, 208, and 1,008 bytes. Compile-time assertions prevent
these sizes from drifting. Within each event-size group, payload bytes and ID
values are identical except for the deliberately selected snapshot-ID variant.
The paired shared case changes only event ownership.

The historical 32-byte single-route event is a separate compact fixture
containing one `u128` snapshot ID and 16 payload bytes. Its owned routed record
was 48 bytes and the corresponding shared routed record was 32 bytes. At this
boundary the two ownership strategies were effectively tied.

Owned cost in the historical matrix scales with both event size and emitted
routes. Shared routed records remain pointer-sized regardless of the underlying
event size.

The current executable Arc-only routing measurements were recorded together in
one unfiltered Criterion run. Every timed operation routes 100 batches of 1,000
input events. Input throughput divides 100,000 by total latency; route
throughput additionally multiplies by the configured routes per event:

| Event bytes | Workers | Routes/event | Total | ns/input | ns/route | Input events/s | Routes/s |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 32 | 8 | 1 | 343.19 µs | 3.4319 | 3.4319 | 291,383,782 | 291,383,782 |
| 32 | 8 | 2 | 902.23 µs | 9.0223 | 4.5111 | 110,836,483 | 221,672,966 |
| 32 | 8 | 3 | 1.5887 ms | 15.8870 | 5.2957 | 62,944,546 | 188,833,638 |
| 64 | 8 | 1 | 738.57 µs | 7.3857 | 7.3857 | 135,396,780 | 135,396,780 |
| 64 | 8 | 2 | 1.4013 ms | 14.0130 | 7.0065 | 71,362,306 | 142,724,613 |
| 64 | 8 | 3 | 2.1455 ms | 21.4550 | 7.1517 | 46,609,182 | 139,827,546 |
| 208 | 8 | 1 | 801.99 µs | 8.0199 | 8.0199 | 124,689,834 | 124,689,834 |
| 208 | 8 | 2 | 1.4978 ms | 14.9780 | 7.4890 | 66,764,588 | 133,529,176 |
| 208 | 8 | 3 | 2.2585 ms | 22.5850 | 7.5283 | 44,277,175 | 132,831,525 |
| 1,008 | 8 | 1 | 720.91 µs | 7.2091 | 7.2091 | 138,713,570 | 138,713,570 |
| 1,008 | 8 | 2 | 1.3842 ms | 13.8420 | 6.9210 | 72,243,895 | 144,487,791 |
| 1,008 | 8 | 3 | 2.1995 ms | 21.9950 | 7.3317 | 45,464,878 | 136,394,635 |
| 64 | 1 | 1 | 584.48 µs | 5.8448 | 5.8448 | 171,092,253 | 171,092,253 |

These cases use the Arc boundary directly rather than wrapping an Arc inside a
second benchmark event type. The full integration matrix contains four exact
event sizes and one, two, and three routes per event, plus the 64-byte
single-worker shortcut. The compact 32-byte event stores a first snapshot ID,
15 payload bytes, and a one-byte route count; compile-time assertions keep its
size fixed while it emits consecutive snapshot IDs. The 64-, 208-, and
1,008-byte fixtures store their snapshot IDs directly.

Input-event throughput necessarily falls as each event fans out further. Route
throughput is the more stable measure of router work: the 64-byte-and-larger
two- and three-route cases sustain approximately 133–144 million emitted
snapshot routes per second. Event payload size has little effect after the Arc
boundary because routed records move the same pointer-sized handle.

When exactly one worker is configured, worker selection returns index zero
without hashing the snapshot ID. The router still uses the same snapshot
visitation, flattening, allocation, completion, and sending logic. The original
pre-Arc-boundary experiment changed the median from 719.47 µs to 580.80 µs, a
19.4% improvement. In the complete current suite, the direct Arc fixture
measures 584.48 µs for one worker and 738.57 µs for eight workers with one
route per event.

Payload construction, initial `Arc` allocation, input-batch Arc cloning, and
worker execution are outside the timed routine. The routing results therefore
measure events already accepted through the Arc-only API and do not include
the cost of initially allocating an `Arc`.

### `Arc::new`

The isolated allocation benchmark constructs each value before timing, then
times only `Arc::new(value)`. Criterion destroys the returned `Arc` after the
timer stops, so these measurements include allocating the reference-counted
block and moving the value into it, but exclude final destruction and
deallocation:

| Value bytes | `Arc::new` |
| ---: | ---: |
| 32 | 13.985 ns |
| 64 | 14.839 ns |
| 208 | 22.550 ns |
| 1,008 | 74.646 ns |

An `Arc` handle is one pointer. On this 64-bit target its allocation also
stores two pointer-sized atomic counters alongside the value, before allocator
metadata and padding.

Worker receivers remain alive but do not process messages during the timed
routine. This keeps the benchmark scoped to the router while allowing
Crossbeam's queue-block allocations to amortize across the 100 batches.
Fixture construction, output destruction, and worker execution are excluded.

### Sustained-routing flame graph

The following historical profile predates the same-event comparison and uses
the earlier 32-byte single-route fixture. The 1,000 Hz, ten-second profile
recorded 10,653 samples. The primary measured
router subtree contained 6,481 samples:

| Router operation | Samples | Share of router subtree |
| --- | ---: | ---: |
| Allocate per-worker destination vectors | 6,000 | 92.6% |
| Send worker batches through Crossbeam | 246 | 3.8% |
| Deallocate consumed input vectors | 139 | 2.1% |
| Remaining routing work | 96 | 1.5% |

Of the 246 Crossbeam-send samples, 236 were in its queue-block allocation.
The benchmark allocates one destination vector per affected worker per batch:
eight workers across 100 batches produce 800 destination-vector allocations.
Each vector reserves 157 routed inputs, so the profile identifies allocation,
not hashing or channel bookkeeping, as the dominant sampled router cost.

These percentages describe samples from an instrumented profiling run, not
the uninstrumented Criterion timing model. They identify the hot call stacks;
the regular benchmark remains the source for absolute latency.
