# ConTime Lanes

`contime-lanes` contains isolated, statically dispatched contracts for routing
canonical events through snapshot-specific filter lanes and apply lanes.

The crate intentionally has no dependency on ConTime or on its API, router,
worker, events, or snapshot subcrates. Core and the root `contime` crate
re-export these contracts for consumer application adapters.

```text
raw lanes -> snapshot filter lanes -> snapshot apply lanes -> snapshot
```

By default, filter and apply lanes use the same borrowed event batch and the
pass-through filter forwards it without allocating or cloning payloads.

## API guide

[`Lanes`](https://github.com/humacss/contime/blob/main/crates/lanes/src/types.rs) defines clonable event views and batch representations.
`FilterLanes::project` selects a snapshot's input from a borrowed canonical raw
batch; `EventFilter::filter` transforms that input into one apply batch. The
`ApplyEvents` implementation consumes that batch synchronously to update state.
`PassThrough` supplies the default filter when both lane families coincide.

Use `project(RawBatch { ... })` followed by `apply` for the checked pipeline.
Every filter must call `FilterOutput::output` exactly once, including when the
result is empty. `apply` panics for zero or multiple outputs; application occurs
inside the output call, so a later panic does not roll back earlier mutations.
The standalone `filter` function delegates to a custom output receiver without
counting its emissions.

Callers supply complete timestamp buckets in canonical order. Projection and
filtering preserve the input snapshot ID, timestamp, and cumulative raw history
count. That count can differ from the number of projected or effective events.
Borrowed output is scoped to application; it need not allocate or clone payloads.
`Route` separately supplies destination identity and initializes otherwise
default snapshot state without advancing its default timestamp.

Generate the public Rust API reference from the workspace root:

```sh
cargo doc -p contime-lanes --no-deps --open
```

## Unit Benchmarks

Criterion measurements from an optimized local run on 2026-08-29:

| Scenario | Events | Median time | Time per raw event | Raw throughput |
| --- | ---: | ---: | ---: | ---: |
| Raw-to-filter projection | 1,000 | 1.0331 us | 1.033 ns | 968.0 million/s |
| Default projection, pass-through, and apply | 1,000 | 1.0259 us | 1.026 ns | 974.8 million/s |
| Decorated projection, filter, and apply | 1,000 raw / 999 output | 2.0171 us | 2.017 ns | 495.8 million/s |

The projection benchmark consumes every borrowed filter-lane event. The
default pipeline additionally invokes the pass-through filter and snapshot
apply dispatch while retaining the same lazy batch representation. Its result
being effectively equal to projection alone indicates that the extra default
boundaries compile away.

The decorated pipeline receives one control event and 999 domain events. Its
filter constructs 999 new output-only apply events in a vector before applying
them, so its measurement includes that allocation and decoration work.

Run the inline benchmarks from the repository root:

```bash
cargo test --release --manifest-path crates/lanes/dev/Cargo.toml \
  filter::tests::benchmark_filter -- --ignored --nocapture

cargo test --release --manifest-path crates/lanes/dev/Cargo.toml \
  apply::tests::benchmark_apply -- --ignored --nocapture
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

### `contime_lanes` · inline · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `lanes/apply/decorated_pipeline_1000_raw_events` | 1.601 µs | 1.596 µs–1.612 µs | 100 | — |
| `lanes/apply/default_pipeline_1000_events` | 807.770 ns | 805.634 ns–808.797 ns | 100 | — |
| `lanes/filter/project_1000_raw_events` | 827.205 ns | 826.523 ns–828.172 ns | 100 | — |
