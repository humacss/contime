# Isolated Router Subcrate Design

## Purpose

Create an independently testable and benchmarkable `contime-router` crate for
input-batch routing. The existing root `contime` crate remains unchanged and
does not use the new crate during this pass.

The router receives complete input batches, maps each input's snapshot IDs to
workers, builds one final batch per affected worker, and dispatches those
batches. It has no dependency on `contime-api` or the root `contime` crate.

## Scope

The router owns only input-batch routing:

- receiving API-independent input batches through a Crossbeam receiver;
- deriving a private AHash state from a caller-provided `u64` seed;
- visiting snapshot IDs emitted by each input;
- hashing every emitted snapshot ID exactly once;
- flattening inputs directly into final per-worker vectors;
- cloning an input only for additional snapshot destinations;
- cloning an opaque completion handle only for additional affected workers;
- sending one batch through each affected worker's Crossbeam sender.

The router does not own:

- its thread;
- channel creation;
- worker construction, shutdown, or processing;
- API message types;
- memory accounting or admission;
- rejection creation or aggregation;
- queries or time advancement;
- response waiting or recovery orchestration.

## Public Types

```rust
pub trait RoutableInput: Clone {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}

pub struct InputBatch<I, C> {
    pub inputs: Vec<I>,
    pub completion: C,
}

pub struct RoutedInput<I> {
    pub snapshot_id: u128,
    pub input: I,
}

pub struct WorkerBatch<I, C> {
    pub inputs: Vec<RoutedInput<I>>,
    pub completion: C,
}

pub enum RouterError {
    NoWorkers,
    WorkerUnavailable { worker_index: usize },
}
```

The completion type `C` is opaque to the router. Production can instantiate it
with a rejection sender; unit tests can use a lightweight token. Routing only
requires `C: Clone`.

`RoutableInput::snapshot_ids` uses a callback to avoid allocating a temporary
snapshot-ID collection. Implementations must emit every relevant snapshot ID
once at most.

## Public Function

The crate exposes one public function:

```rust
pub fn route<I, C>(
    seed: u64,
    input: Receiver<InputBatch<I, C>>,
    worker_outputs: &[Sender<WorkerBatch<I, C>>],
) -> Result<(), RouterError>
where
    I: RoutableInput,
    C: Clone;
```

The orchestrator creates all channels and runs `route` on its chosen thread.
The function derives one AHash state from `seed`, then receives and routes
batches until the input channel disconnects. A disconnected input channel is a
normal shutdown and returns `Ok(())`.

The same seed, worker count, snapshot IDs, and router algorithm version always
produce the same worker assignments. Changing the seed intentionally remaps
snapshot IDs.

## Batch Routing Algorithm

For each received batch:

1. Allocate an outer worker-slot vector sized exactly to the worker count.
2. Estimate an initial inner-vector capacity from input count divided by worker
   count, plus a small margin.
3. Consume inputs in caller order.
4. For each input, call `snapshot_ids` once.
5. Hash each emitted snapshot ID once and append a `RoutedInput` directly to
   that worker's final vector.
6. Retain one pending snapshot ID while visiting so an input with `N`
   destinations is cloned `N - 1` times and moved into the final destination.
7. Count affected workers.
8. Clone the completion handle `affected_workers - 1` times and move the
   original into the final worker batch.
9. Send exactly one `WorkerBatch` to each affected worker.

The router never constructs an intermediate globally flattened vector.

An empty batch, or a batch whose inputs emit no snapshot IDs, sends no worker
messages. Its completion handle is dropped when batch processing ends.

## Errors

An empty worker-output slice returns `RouterError::NoWorkers` before receiving
input.

If a worker output disconnects, routing returns
`RouterError::WorkerUnavailable { worker_index }`. Batches already dispatched
to earlier workers remain dispatched. The router does not attempt rollback or
construct rejections; partial-dispatch recovery belongs to the orchestrator.

## Source Structure

```text
crates/router/
├── .gitignore
├── Cargo.toml
├── README.md
└── src/
    ├── lib.rs
    ├── route.rs
    └── types.rs
```

`types.rs` contains all public types and the routing trait. `route.rs` contains
the public function, its private dependency seam, inline unit tests, and inline
Criterion benchmark. `lib.rs` only declares modules and re-exports the public
interface.

The crate remains outside the root workspace during this isolated pass. There
is no integration-test directory.

## Testing

Inline unit tests cover:

- normal shutdown when the input channel disconnects;
- rejection of an empty worker-output slice;
- stable worker assignments for the same seed across separate route runs;
- one routed record per emitted snapshot ID;
- preservation of input order within each worker batch;
- exactly `N - 1` input clones for `N` snapshot destinations;
- exactly one completion handle per affected worker;
- no worker output for inputs without snapshot IDs;
- propagation of the failed worker index when an output disconnects.

The production worker send is hidden behind a private file-local dependency
trait so individual routing behavior can be unit tested without worker
internals.

## Benchmarking

The initial inline Criterion benchmark routes one received batch containing
1,000 single-destination inputs across eight real Crossbeam worker outputs.
It includes:

- input-channel receipt;
- hasher derivation;
- snapshot-ID visitation and hashing;
- outer and inner routing-vector allocation;
- direct flattening into worker batches;
- completion-handle cloning per affected worker;
- worker-channel dispatch.

It excludes input construction, channel construction, and worker processing.
The README records the resulting confidence interval and exact benchmark
command after verification.
