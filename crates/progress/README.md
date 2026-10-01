# contime-progress

Policy-independent reduction of one bounded, fenced progress measurement.
No threads, queues, storage, pruning, or dependencies.

`MeasurementRound` captures a round ID, cutoff and fixed nonzero participant
count. Each participant reports its earliest possible remaining work, or `None`
when it has none. The final unique report returns the minimum, capped at the
cutoff. Wrong-round, duplicate and unknown-participant reports are ignored.
Missing reports never count as completion.

`constrain(boundary)` conservatively lowers an unfinished observation when the
caller discovers additional work. Core uses its previously committed safe
boundary here when an admission can reconstruct an older checkpoint prefix.

Results are exclusive boundaries and preserve the caller's timestamp ordering,
including substeps. Observations in separate rounds need not increase; the
caller owns a monotonic committed frontier and decides when it is safe to move.

## Responsibilities of the caller

- Establish ordered fences before accepting participant reports.
- Account for admissions and work in transit throughout the round.
- Use unique round IDs and a fixed, correctly mapped participant set.
- Resolve/resume participants even when no progress is possible.
- Decide what to do with the result: prune, publish an observation, or wait.

This crate alone does not implement a distributed completion protocol. A Stream
adapter must include delivery and membership accounting, not just take the
minimum of stale runtime observations.

## Current reuse

Core's pruning `Frontier` delegates measurement to this crate while retaining
its requested/safe timestamps, round sequencing and monotonic pruning policy.
The existing router fences and worker callbacks remain unchanged. Core's
`start_with_progress` also accepts this crate's `ProgressPolicy`, selecting a
separate observation cutoff and response without increasing the prune target.
Both consumers share one serialized measurement round, rather than pausing the
same workers through competing coordinators.

`ProgressObservation` carries the round ID, requested cutoff (as capped by the
host) and measured exclusive boundary. Reports in successive rounds may fall
back when newly admitted work requires earlier processing. Engine/Stream must
account for in-flight work before publishing a monotonic group-safe boundary.
Policies that cache reports can implement the default `invalidated()` callback.
Core invokes it before routing admitted batches, including internal feedback,
so a completed insertion cannot leave a pre-admission report in that cache.
It does not account for messages still outside Core or replace fenced reports.

## Verification

```sh
cargo +1.95.0 test --offline --manifest-path crates/progress/Cargo.toml
cargo +1.95.0 test --offline --manifest-path crates/core/Cargo.toml
```

Seven public-API tests cover missing and stale reports, duplicate completion,
cutoffs, pending work, concurrent-admission constraints, complete timestamps,
and independent pruning/observation consumers of the same implementation.
