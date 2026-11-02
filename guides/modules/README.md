# Application modules

[Overview](../../README.md) · [Explicit traits](../usage/README.md) · [Time](../../crates/time/README.md)

ConTime 0.1.2 provides optional helpers through `features = ["macros"]`.
Start with [`examples/readme.rs`](../../examples/readme.rs); each teaching
example is one standalone file, separate from the launcher workloads.

## Identity and timestamps

Events and snapshots can mark one directly stored `u128` identity field:

```rust,ignore
#[derive(ContimeEvent)]
struct OrderedValue {
    #[contime(id)]
    event_id: u128,
    list_id: u128,
    #[contime(time)]
    time: i64,
    value: char,
}
```

The field shorthand accepts exactly one ID field, on either an event or snapshot. It cannot be combined with a
struct-level selector or `map`. For more complex event or snapshot identities,
select a field or tuple of fields on the struct and optionally map that selection:

```rust,ignore
#[contime(
    id = |value| (value.owner_id, value.list_id),
    map = |fields| make_content_id(fields),
)]
```

The selector is declarative: one direct field or a nonempty tuple of direct
fields. Use `&value.name` for non-Copy fields that a mapper should borrow. Mapping
receives the selected value/tuple and must return `u128`; without `map`, the
selection must already be `u128`. All computation belongs in the mapper.
`map` requires an explicit struct-level selector. A standalone function receiving
`&Self` is also supported for an opaque identity implementation without mapping;
it cannot supply the source fields needed for inferred registration.

The [composite-ID example](../../examples/composite_ids.rs) uses Content ID 1.0
explicitly. The macro never chooses a hash algorithm or namespace. Ensure IDs
are unique across event types and snapshot types. Type-name namespaces work for
local examples; use stable application namespaces when IDs must survive renames.

Time access is independent. `#[contime(time)]` selects one named or tuple field.
Events clone its value; snapshots borrow it and assign it through `set_time`.
The field type becomes `Time`, and modules infer it from snapshots and check all included types for agreement. Supply snapshot `Clone` yourself. No payload Send/Sync bounds are added.

## Event registration and initialization

```rust,ignore
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(events = [OrderedValue, ValueRemoved])]
#[contime(event = ForwardedValue, id = |value| value.destination)]
struct OrderedList {
    #[contime(id)]
    list_id: u128,
    #[contime(time)]
    time: i64,
    values: Vec<char>,
}
```

`events = [...]` is a bracketed list of event types using the snapshot's defaults.
`event = Type` registers one event, optionally overriding its `id` field selector,
`map`, or both. An omitted option inherits the snapshot's option, including the
absence of mapping. A registration-only `map` can use the inherited selector.
Customization inside a plural registration is rejected; use singular attributes.
Both forms may be mixed, and repeated spellings of an event type are errors.
Aliases of the same concrete type cannot be resolved syntactically; the builder
keeps the first registration. Use consistent names and spellings.

Default registrations read the same named fields from the event. Overrides select
source fields in the same order and tuple shape as the snapshot's identity fields.
Their types must match. On first creation, those source fields are cloned into
the snapshot's identity fields; other fields come from `Default`. Routing never
constructs a temporary snapshot. Borrowed selectors do not clone identity fields
while routing or mapping; creation clones them once into owned state.

Overrides must produce the ID of the initialized snapshot. ConTime checks that
invariant before accepting created state. The
[registration walkthrough](../../examples/event_registration.rs) demonstrates defaults,
renamed source fields and equivalent alternative mapping.

The derive implements `SnapshotEvents` and `SnapshotCreate` independently from
application. Automatic initialization requires snapshot `Default` and clonable
identity fields. The macros do not implement Clone or Default themselves.

## Application

```rust,ignore
impl SnapshotApply for OrderedList {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<OrderedValue>() {
            self.values.push(event.value);
        }
    }
}
```

`apply_batch` has an empty default: `impl SnapshotApply for OrderedList {}` opts
into recording history and maintaining time without application-specific state
updates. Add the method later to supply application logic. No application method
attribute is needed.

Each call handles one complete effective timestamp bucket. Typed views borrow
events and can be read repeatedly. Handle different event types in an explicit
phase order chosen by your application. Each view preserves canonical ordering;
ConTime never merges distinct effective timestamps into the same application.
`ApplyContext` also exposes the snapshot ID, effective time and raw canonical
history event count preceding this batch. Query and retention reconstruction use
the same deterministic application logic.

## Handwritten implementations

Every helper is optional. Omit generated identity/time implementations when you
provide their traits manually. Without registration attributes, implement
`SnapshotEvents::define` using `EventSetBuilder` and `SnapshotCreate::create`
using the first `EventRef`; keep application on `SnapshotApply`.

```rust,ignore
impl SnapshotEvents for Counter {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<Changed>(|event, emit| {
            for id in &event.targets { emit(*id); }
        });
    }
}
impl SnapshotCreate for Counter {
    fn create(id: u128, _: EventRef<'_>) -> Self {
        Self { id, ..Self::default() }
    }
}
```

This path supports zero/many destinations, specialized initialization, and state
without Default. The earlier explicit emitter form
`#[contime(event = Changed, id = emit_counter_ids)]` remains available for custom
routing with manual `SnapshotCreate`; it cannot be mixed with inferred registrations.
For new custom models, handwritten registration keeps the contract explicit.
The [explicit-traits example](../../examples/explicit_traits.rs) also shows the
lower-level Input/ApplyEvents and Module contracts with no macros at all.

## Compose once and start one instance

```rust,ignore
#[derive(ContimeModule)]
#[contime(snapshots = [Counter])]
struct Counters;

#[derive(ContimeModule)]
#[contime(snapshots = [CounterStatistics])]
#[contime(modules = [Counters])]
struct Statistics;

#[derive(ContimeModule)]
#[contime(modules = [Counters, Statistics])]
struct App;

let contime = ConTime::<App>::start(config)?;
```

The module infers its time from its first snapshot or child module. Events are
collected from each snapshot's registration metadata, flattened and deduplicated.
All snapshots, events and child modules must agree on time; mismatches and cyclic
dependencies are compile errors. A source-free module needs an explicit
`#[contime(time = i64)]` because there is no type from which to infer time.

Snapshot derives export hidden definition-site metadata, supporting paths across
crates, renamed ConTime dependencies, generic snapshots and default generic
arguments. No source scanning or runtime registration registry is used. The
[composition walkthrough](../../examples/modules.rs) shows a shared dependency
included once through multiple paths.

### Snapshot-specific event subsets

```rust,ignore
#[derive(ContimeModule)]
#[contime(snapshot = Counter, events = [Added, Removed])]
#[contime(snapshots = [AuditLog])]
#[contime(modules = [Shared])]
struct App;
```

Plural snapshots use defaults. Singular `snapshot = Type` also uses defaults
unless `events = [...]` selects a subset of that snapshot's supported types.
The subset is checked against the snapshot metadata and is local to this module
registration. An event included for AuditLog does not reach Counter if Counter
excludes it. An empty subset is valid and routes no events to that snapshot.

When the same snapshot type/name appears through several modules, its subsets
are unioned. A full/default contribution includes all supported events. Combining
modules merges one shared history definition; it does not create separate copies
with competing policies. See [`module_subsets.rs`](../../examples/module_subsets.rs)
for a runnable demonstration of filtering while another snapshot receives the
excluded event.

Generated `AppEventLanes` accepts `.into()` conversions from included events.
Queries return `AppSnapshotLanes`, with one variant per included snapshot type.
Use `ManualConTime::<App>` with `manual` enabled for caller-driven processing;
these definitions work with both execution modes.

### Explicit metadata for handwritten snapshots

If registration is handwritten and not declared on a snapshot derive, list the
event types explicitly:

```rust,ignore
#[derive(ContimeModule)]
#[contime(events = [Added, Removed])]
#[contime(snapshots = [HandwrittenCounter])]
struct App;
```

This escape hatch builds event lanes without snapshot metadata callbacks. At
startup, the actual handwritten EventSet validates its supported event types and
any requested subset. For large applications, wrap handwritten definitions in
such a module and compose it with inferred modules. Supplying explicit event
metadata is distinct from restricting a singular snapshot registration.
The previous `#[contime_module(time = ..., events = ..., snapshots = ...)]`
syntax remains compatible. The fully handwritten
[`explicit_traits.rs`](../../examples/explicit_traits.rs) uses no derives at all.

## Registration names and aliases

A module lists snapshots and child modules, inferring event types by default. Child
registrations are included automatically; ConTime accepts one final module.
The macro generates flat enums and deduplicates repeated registration names,
including shared dependencies reached through several modules. It emits Rust
type-equality checks before merging registrations with the same name.

A bare type uses the last part of its path as its registration and variant name.
Keep that name consistent across the composed application. For an import alias,
retain the shared registration name explicitly:

```rust,ignore
use Added as Incoming;
#[derive(ContimeModule)]
#[contime(events = [Added = Incoming])]
#[contime(snapshots = [HandwrittenCounter])]
struct App;
```

The same `Name = Type` syntax works in `snapshots` and singular event subsets. Distinct types with the same
short name, or distinct generic specializations, need different names, for example
`events = [Small = Number<u32>, Large = Number<u64>]`.

**Known limitation:** the macro cannot discover that different aliases name the
same Rust type. Registering it under different names prevents deduplication and
causes conflicting generated `From` implementations at compile time. Use one
registration name per concrete type; no duplicate lanes run silently.

Inferred snapshot lookup uses its definition path to find the companion metadata.
If you rename/import a snapshot under another name, use its qualified definition
path for inference, or use explicit event metadata. Opaque type aliases do not
carry an independently discoverable companion macro. Snapshot event aliases must
also use consistent registration names across the application.

## Time and observations

The default `time` feature implements common integer timestamps. `i64` uses its
full signed range, with no special boundary at zero. Retention must be nonnegative
and uses the application's chosen units. Without built-ins, implement
`time::Timestamp` and `time::AdvanceTime` for your own type and explicitly provide
its minimum. Runtime and Manual both start at that minimum.

Submission queues work. Queries observe retained admitted history and do not wait
for global idle. The example uses a request-scoped admission response; global idle
waits remain test/benchmark helpers. Starting a composed module starts one instance,
not one worker topology for each dependency.

## Updating from 0.1.1

The module API changes the public construction types. Use
`ConTime::<App>::start(config)` or `manual::ManualConTime::<App>::new(config)`
when supplying a module. Existing implementations that directly supply event and
snapshot lanes can keep those contracts and rename their instances to
`RawConTime<I, S, W>` or `manual::RawManualConTime<I, S, W>`; their wrapper arguments
remain explicit.

Custom time types now implement `Timestamp::minimum()` and
`AdvanceTime::is_valid_retention()` in addition to predecessor and retention
subtraction. The minimum is the true domain lower bound, not `Default` or zero.
Retention intervals must be nonnegative. For built-in integer time, leave the
`time` feature enabled; a Manual-only integer configuration uses
`default-features = false, features = ["manual", "time"]`. Omit `time` only when
providing the time traits for an application-owned type.
