# ConTime Lanes

`contime-lanes` contains isolated, statically dispatched contracts for routing
canonical events through snapshot-specific filter lanes and apply lanes.

The crate intentionally has no dependency on ConTime or on its API, router,
worker, events, or checkpoints subcrates. The eventual ConTime orchestrator
will adapt those independently designed boundaries.

```text
raw lanes -> snapshot filter lanes -> snapshot apply lanes -> snapshot
```

By default, filter and apply lanes use the same borrowed event batch and the
pass-through filter forwards it without allocating or cloning payloads.

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
cargo test --release --manifest-path crates/lanes/Cargo.toml \
  filter::tests::benchmark_filter -- --ignored --nocapture

cargo test --release --manifest-path crates/lanes/Cargo.toml \
  apply::tests::benchmark_apply -- --ignored --nocapture
```
