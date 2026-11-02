# contime-macros

The separately published implementation of ConTime's optional `macros` feature.
Import helpers from `contime::prelude::*`; the normal traits remain usable without
macros. See the [application guide](../../guides/modules/README.md) for the full API.

## Identity and time helpers

```rust,ignore
#[derive(ContimeEvent)]
struct Added {
    #[contime(id)]
    event_id: u128,
    list_id: u128,
    #[contime(time)]
    time: i64,
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(events = [Added])]
struct List {
    #[contime(id)]
    list_id: u128,
    #[contime(time)]
    time: i64,
}
impl SnapshotApply for List {}
```

One `#[contime(id)]` field on an event or snapshot is shorthand for a directly stored u128.
Complex event/snapshot identities use a struct-level selector and optional mapper:

```rust,ignore
#[contime(id = |v| (v.owner, v.list), map = |fields| custom_id(fields))]
```

Selectors expose direct fields or a tuple of fields; borrow non-Copy sources with
`&v.name`. Mapping receives that selection and returns u128. Map cannot be supplied
without a struct-level selector or combined with field-ID shorthand. Ordinary
standalone identity functions taking &Self remain supported without mapping.
The helpers preserve generics and impose no payload Send/Sync requirements.

## Event registration and creation

`#[contime(events = [Added, Removed])]` registers a bracketed list with the snapshot's
selector/map. Singular `#[contime(event = Forwarded, id = |v| v.destination)]`
overrides source fields; `map` can also be overridden. Missing options inherit
the snapshot's setting. Plural registrations cannot contain overrides.

The derive emits `SnapshotEvents` and `SnapshotCreate`: event identity fields are
copied to corresponding snapshot identity fields, with remaining state defaulted.
Selections must have matching shape and field types. Snapshot Default and identity
field Clone are required for automatic creation. No temporary snapshot is built
while routing. Final created IDs are checked against the emitted destination.

Application remains an ordinary `SnapshotApply` implementation. Its default method
is empty; override `apply_batch` for deterministic state updates. There is no
application attribute. Manual `SnapshotEvents`, `SnapshotCreate` and `SnapshotApply`
remain available for custom models. Explicit emitter callbacks in the singular
id option are retained with manual creation; do not mix them with inferred events.
Repeated spelled event types are rejected; distinct aliases retain the builder's
first-registration rule. Use consistent spellings.

## Modules

```rust,ignore
#[derive(ContimeModule)]
#[contime(snapshots = [List])]
#[contime(snapshot = Audit, events = [Added])]
#[contime(modules = [Shared])]
struct App;
```

`ContimeModule` collects snapshot metadata and child modules into flat lanes,
inferring events and time. Plural snapshots use defaults; singular registrations
can select a supported event subset. Filters are per snapshot and cached at
startup, so an event admitted for another snapshot cannot leak into a restricted
one. Duplicate snapshot contributions union subsets; a full contribution dominates.
Time mismatches, unsupported subsets and cycles are compile errors.

Handwritten snapshots can provide explicit `#[contime(events = [...])]` module
metadata. The old `contime_module` attribute remains compatible. Inferred snapshot
references use their qualified definition path; opaque aliases need explicit
metadata or the original path. See [module composition](../../examples/modules.rs)
and [event subsets](../../examples/module_subsets.rs).

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

`src/lib.rs` contains narrow derive/collector entry points. `helpers/field.rs`
handles markers, `options.rs` parses independent attributes, `selector.rs` validates
source selections and mapping, `identity.rs` implements ID traits, `events.rs`
generates registration, and `creation.rs` emits initialization. `expand.rs`
combines these optional helpers.

`attributes.rs` parses module lists and names; `expansion.rs` resolves the consumer
crate, including renamed dependencies; `registration.rs` creates module keys.
The `module/` units own definitions, metadata traversal, subset merging and flat lane
generation. Snapshot `helpers/metadata.rs` exports concrete event aliases, with
generic parameter/default handling isolated from routing and initialization.
