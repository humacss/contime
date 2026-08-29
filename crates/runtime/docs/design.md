# ConTime Runtime Design

## Purpose

`contime-runtime` owns the process-local execution topology for ConTime's
apply path. It starts router and worker threads, connects them with channels,
and coordinates their shutdown.

The crate is initially isolated. It does not depend on `contime` or any other
ConTime subcrate. Instead, it declares the minimal router and worker traits it
needs. The eventual root orchestrator will adapt the independently designed
API, router, worker, events, checkpoints, and lanes crates to these contracts.

## Scope

The first version supports apply execution only:

- start a configurable number of router threads;
- start a configurable number of worker threads;
- distribute complete apply inputs among routers through one shared queue;
- give every router access to every worker input queue;
- expose the apply-input boundary to the caller;
- close the pipeline gracefully; and
- join all threads while reporting failures and panics.

Queries, advanced/time operations, automatic restarts, failure recovery,
domain messages, routing policy, memory policy, replay logic, and snapshot
logic are outside this crate.

## Isolation Boundary

The runtime treats all messages as opaque generic values. It must not inspect,
clone, route, apply, replay, or otherwise interpret them.

The runtime declares local `Router` and `Worker` traits. Implementations own
their receive loops and must return after their input receiver disconnects.
The runtime knows only how to create their channels, place their `run` calls on
threads, and collect their outcomes.

Conceptually, the contracts are:

```rust
pub trait Router: Send + 'static {
    type Input: Send + 'static;
    type WorkerInput: Send + 'static;
    type Error: Send + 'static;

    fn run(
        self,
        input: Receiver<Self::Input>,
        workers: Vec<Sender<Self::WorkerInput>>,
    ) -> Result<(), Self::Error>;
}

pub trait Worker: Send + 'static {
    type Input: Send + 'static;
    type Error: Send + 'static;

    fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error>;
}
```

Runtime startup statically requires the router's `WorkerInput` type to equal
the worker's `Input` type.

## Configuration

`RuntimeConfig` contains execution-topology settings owned by the runtime:

- `router_count`;
- `worker_count`.

Both counts must be nonzero. Invalid configuration fails before channel
allocation or thread startup.

Threads receive deterministic runtime-owned names containing their role and
stable index. Custom names, stack sizes, affinities, and scheduling policies
are deferred until a concrete need appears.

Routing seeds and domain-specific router or worker settings are not runtime
configuration. Callers capture those settings in their router and worker
factories.

## Startup

`Runtime::start` receives a runtime configuration, a router factory, and a
worker factory. Each factory receives the stable zero-based index of the
instance it creates.

Startup performs these operations:

1. Validate configuration.
2. Create one shared apply-input channel.
3. Create one private input channel for each worker.
4. Construct and start every worker.
5. Construct and start every router, cloning the shared input receiver and
   worker senders for each router.
6. Drop startup-only channel handles.
7. Return a running `Runtime` handle.

The routers compete on the shared receiver. The first available router takes
the next complete message. Messages are never split by the runtime.

If a thread cannot be spawned, startup closes all channels already created,
joins every thread already started, and returns a structured startup error.
No started thread is intentionally detached during startup rollback.

## Running Handle

The running handle owns:

- the apply-input sender;
- router join handles; and
- worker join handles.

It provides a direct `send` convenience method and borrowed access to the
input sender for adapters that already operate on channels. Callers may clone
that sender, but graceful shutdown cannot finish until all external sender
clones have been dropped.

Completion and rejection semantics remain inside the opaque input message.
The runtime never waits for, combines, or interprets request responses.

## Shutdown

Explicit shutdown consumes the runtime and proceeds in dependency order:

1. Drop the runtime-owned apply sender.
2. Join every router.
3. Once all routers have returned, their worker-sender clones are gone and
   every worker input channel closes.
4. Join every worker.
5. Return a complete shutdown report.

Joining never returns early. The report preserves an ordered outcome for each
configured router and worker. Each outcome distinguishes successful
completion, a returned implementation error, and a thread panic.

Dropping a runtime without explicit shutdown closes its owned apply sender,
but cannot return thread outcomes. Explicit shutdown is the supported path
when the caller needs proof that every thread terminated.

## Failure Semantics

This version observes failures but does not supervise or recover from them.
A router or worker that returns early simply records its outcome for shutdown.
Other instances continue until their channels close or they independently
return.

The runtime does not infer that one failed stage requires cancellation of all
other stages. Coordinated cancellation, restart policies, and live health
reporting are deferred until their required behavior is understood.

## Source Layout

- `src/types.rs`: configuration, traits, errors, outcomes, and reports.
- `src/start.rs`: validation, channel construction, factories, spawning, and
  partial-start rollback.
- `src/runtime.rs`: the running handle and apply-input access.
- `src/shutdown.rs`: dependency-ordered closure and complete joining.
- `src/lib.rs`: public exports only.

## Verification

Inline unit tests will use local fake implementations and cover:

- rejection of zero routers or workers before startup;
- stable router and worker factory indexes;
- shared-queue distribution where each input is consumed once;
- router-to-worker forwarding without runtime message inspection;
- clean channel-driven shutdown;
- collection of all returned errors and panics; and
- cleanup of already-started threads when later startup fails, where the
  standard thread API permits deterministic fault injection.

Unit benchmarks should measure only meaningful runtime-owned operations such
as startup/shutdown overhead. An end-to-end integration benchmark with no-op
router and worker implementations is deferred until the functional contract
is stable. Real ConTime adapters and domain work are explicitly excluded from
this isolated crate's measurements.
