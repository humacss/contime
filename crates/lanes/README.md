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

## Unit benchmarks

Inline benchmarks measure borrowed raw-to-filter projection, the default
pass-through pipeline, and filtering that allocates decorated apply events.
Run them from the repository root:

```sh
cargo test --release -p contime-lanes filter::tests::benchmark_filter -- --ignored --nocapture
cargo test --release -p contime-lanes apply::tests::benchmark_apply -- --ignored --nocapture
```
