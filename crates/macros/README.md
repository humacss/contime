# contime-macros

Derives used by the `macros` feature of [`contime`](https://crates.io/crates/contime).
Use the re-exports from `contime`; this crate is the separately published
procedural macro implementation.

```rust,ignore
use contime::{ContimeEvent, ContimeSnapshot, ContimeModule};

#[derive(ContimeEvent)]
struct Added { time: i64, counter_id: u64, amount: i64 }

#[derive(ContimeSnapshot)]
#[contime_snapshot(
    id = [self.counter_id],
    events = [Added],
    apply = {
        for event in batch.events {
            let CounterEvent::Added(event) = event;
            self.value += event.amount;
        }
    }
)]
struct Counter { time: i64, counter_id: u64, value: i64 }

#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added], snapshots = [Counter])]
struct Counters;
```

`ContimeEvent` implements timestamp access and a namespaced, full-width content
identity. It implements `Hash` over all named fields by default. Set
`#[contime_event(id = self.logical_id)]` to hash a custom expression without
requiring the other fields to implement `Hash`. Use `hash = false` when supplying
your own `Hash` implementation for the default whole-event identity. A custom
`time = self.timestamp` selects a timestamp field other than `time`.

`ContimeSnapshot` implements `Clone` and `Default` by default. Set `clone = false`
and/or `default = false` to use handwritten implementations. `id` names the stable
logical identity fields. `Counter::snapshot_id(counter_id)` constructs the same
identity used by generated routes. Snapshot types provide separate identity
namespaces even when their logical fields match.

Each entry in `events` defaults to reading the event fields named by `id`.
Override those fields with `Other => [self.other_id]`, or emit multiple routes
with `Multi => each(self.targets, |target| [*target])` for scalar IDs. Iteration
borrows each collection item. Compound destinations can combine event and item
fields, such as `Multi => each(self.targets, |target| [self.owner, *target])`.
The routing expressions'
`self` is the event. Initial state comes from `Default`, with its identity fields
copied from the first matching route. Other initial fields remain their defaults.

The application body receives `batch.snapshot_id`, `batch.time`,
`batch.history_event_count`, and `batch.events`, an iterator over a generated
`CounterEvent<'_>` enum. Each variant contains a borrowed event; payloads need no
`Clone` or `Copy` implementation.

Modules generate `CountersEventLanes` and `CountersSnapshotLanes`. Both offer
`From` conversions from their constituent concrete types. Add nested dependencies
with `modules = [domain::OtherModule]`; shared modules and concrete lanes are
deduplicated. Every module declares `time = Type`, and all nested declarations,
events, and snapshots must agree on that type. Dependency cycles are errors.
Exported metadata supports independent provider crates and renamed `contime`
dependencies, including repeated `macro_rules!` definitions in separate modules.

Event and snapshot paths can use Rust imports and renamed imports. Dependency
module paths across Rust namespaces currently need the definition path (for
example `domain::Counters`), rather than a bare imported or renamed marker.
Modules declared in the same namespace can use each other's names directly;
renaming the enclosing Rust module is supported. Module markers
must be concrete, while event and snapshot types may be generic. Distinct concrete
lanes that request the same enum variant name are rejected; import one lane under
a distinct name. Identity registration for generic arguments uses their written
arguments, with a compiler-checked type equality guard before deduplication.
Use a consistent spelling across module declarations for shared generic lanes. Rust type
equality checks verify every deduplication candidate; identical argument spellings
that resolve to different concrete types are compile errors. Borrowed event enum
bounds must not refer to snapshot parameters absent from the event types.

## Implementation layout

`src/lib.rs` contains the procedural macro entry points. `attributes.rs` parses
configuration and validates named fields; `expansion.rs` resolves the consumer's
crate name and converts errors to compiler diagnostics. `event.rs` generates
event implementations, while `registration.rs` provides nominal definition keys.

Snapshot expansion lives in `snapshot/`: `definition.rs` validates identity fields
and generates automatic traits, `routing.rs` handles destinations,
`borrowed_events.rs` builds borrowed payload views, and `expand.rs` assembles them.
Module composition lives in `module/`: `metadata.rs` owns the callback protocol,
`composition.rs` traverses dependencies, and `lanes.rs` generates aggregate lanes.
Unit tests and expansion benchmarks stay with the code they exercise; application
and compile-failure coverage lives in the root package's `tests/modules/` suite.
