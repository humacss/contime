# contime-macros

Module derive used by the optional `macros` feature of
[`contime`](https://crates.io/crates/contime). Import the derive and application
traits from `contime`; this package is the separately published procedural macro
implementation.

`ContimeModule` is the only ConTime-specific derive. Events implement
`contime::checkpoints::Event` and `contime::EventId`. Snapshots implement
`contime::checkpoints::Snapshot`, `contime::SnapshotId` and
`contime::SnapshotApply`, using ordinary Rust `Clone` and optional `Default`.
Identity, initialization, timestamp access and application remain ordinary traits.
Register accepted events and their snapshot-ID callbacks in the snapshot builder:

```rust,ignore
use contime::prelude::*;

impl SnapshotApply for Counter {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<Added>(|event, emit| emit(event.counter_id));
    }

    fn create(snapshot_id: u128, _event: EventRef<'_>) -> Self {
        Self { counter_id: snapshot_id, ..Self::default() }
    }

    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Added>() {
            self.value += event.amount;
        }
    }
}
```

A routing callback may emit zero, one or multiple final IDs. The ID returned by
`SnapshotId::snapshot_id(&self)` must match the selected destination. ConTime
passes IDs through unchanged and chooses no hashing algorithm or namespace.
Creation can inspect its first event with `event.get::<Added>()`; routing does
not construct temporary snapshots.

`ApplyContext` exposes `snapshot_id()`, `time()`, `history_event_count()` and
repeatable typed event views. One batch is one effective timestamp. Handle each
event type in your own explicit phase order, preserving canonical ordering
within each type. Payloads are borrowed without cloning. Small buckets retain
references inline; larger buckets allocate reference storage. Event types must
be `'static`; the borrowed values do not need to live forever or implement
`Clone`, `Send` or `Sync`.

See the [application guide](../../guides/modules/README.md) and
[ordered-list walkthrough](../../examples/ordered_list.rs) for complete definitions,
identity rules, multiple destinations, initialization and execution. The prelude
re-exports commonly used types according to enabled features.
`ContimeModule` composes lanes and enforces their shared time type. Add dependencies
with `modules = [domain::OtherModule]`; shared modules and lanes are deduplicated.
Cycles are errors. Exported metadata works across provider crates and renamed
`contime` dependencies. Dependency module paths across Rust namespaces use the
definition path rather than a renamed marker. Module markers are concrete empty
unit structs; event and snapshot types may be generic. ConTime accepts one final
module; dependencies contribute flat event and snapshot lanes.

## Registration names and aliases

A listed type uses its final path identifier as its registration and enum variant
name. Use consistent names for shared registrations. For aliases, write
`events = [Added = Incoming]`; `snapshots` supports the same `Name = Type` syntax.
Distinct types with the same short name and distinct generic specializations need
different names, such as `events = [Small = Number<u32>, Large = Number<u64>]`.
Rust type-equality checks reject merging unrelated types under the same name.

The macro cannot discover that different aliases refer to the same concrete type.
Registering one type under different names causes conflicting generated `From`
implementations at compile time. This is a known limitation; use a consistent
registration name rather than expecting automatic alias deduplication.

## Implementation layout

`src/lib.rs` contains only the module derive and hidden collector entry points.
`attributes.rs` parses explicit lists and registration names; `expansion.rs`
resolves the consumer crate. `registration.rs` creates opaque module identity keys.
`module/metadata.rs` exports definition-site aliases, `composition.rs` traverses
nested modules, and `lanes.rs` generates flat enums and trait adapters. No
per-event or per-snapshot metadata macro is required.
