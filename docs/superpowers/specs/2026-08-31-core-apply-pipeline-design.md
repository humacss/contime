# Core Apply Pipeline Design

## Scope

Build the smallest complete apply-only `contime-core` crate by composing the
isolated API, router, runtime, worker, event-history, checkpoint, lanes, and
memory contracts. Query, advance, horizon pruning, macros, integration
benchmarks, and transactional cross-worker admission remain out of scope.

## Public boundary

The crate exposes a running `ConTime` instance, configuration, an owned input
trait, a snapshot trait boundary compatible with checkpoint replay, memory
usage inspection, synchronous `apply`, and shutdown. Consumers submit owned
events. Core performs conservative batch admission, wraps accepted events in
tracked shared ownership, and forwards one batch through the API boundary.

## Data flow

1. Core checks the raw batch's conservative tracked size against the usable
   memory budget.
2. Accepted events become tracked shared events exactly once.
3. An API adapter message carries the events and request completion sender.
4. Router adapters fan each shared event out to its snapshot IDs and worker.
5. Worker adapters insert routes into canonical event histories.
6. Scheduled replay mutably acknowledges event history, materializes
   checkpoints, and completes request waiters.
7. API completion is observed when all completion senders are dropped; only
   rejected event IDs are transmitted.

## Memory accounting

Core owns one cloneable atomic budget. Tracked shared events account for the
underlying event allocation once and every live pointer. Checkpoint state uses
tracked owned storage so cloning owns an independently mutable snapshot and
replay reports snapshot-size deltas. The configured safety buffer is excluded
from ordinary admission. A whole input batch is rejected before wrapping when
its conservative event size does not fit. Internal collection spare capacity
is not separately estimated in this first pass; the safety buffer covers that
implementation overhead.

## Isolation

`contime-core` depends only on sibling subcrates and third-party libraries; it
does not import the root `contime` crate. Adapter structs implement the output
trait of one crate and the input trait of the next, avoiding conversion passes
at the boundaries.

## Verification

Each executable source unit contains focused unit tests and one ignored inline
Criterion benchmark for its hot public path. This pass adds no integration
tests or integration benchmarks. Focused crate tests, all-target checks,
formatting, and diff validation are required before handoff.
