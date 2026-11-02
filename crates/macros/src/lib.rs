//! Static module composition and optional time and ID helpers over ordinary traits.
use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};
mod attributes;
mod expansion;
mod helpers;
mod module;
mod registration;
/// Assembles snapshots and nested modules into flat event and snapshot lanes.
///
/// Use `#[contime(snapshots = [Counter])]` for default registrations and
/// `#[contime(snapshot = Audit, events = [Added])]` for a supported event subset.
/// Add dependencies with `#[contime(modules = [Shared])]`. Snapshot metadata
/// supplies event types; time is inferred from the first snapshot or child module,
/// with compile-time checks that all included types agree.
///
/// Event subsets are local to one snapshot's cached registration. Contributions
/// of the same snapshot union their subsets, with a full registration dominating.
/// Shared module dependencies are included once and cycles are rejected.
/// Handwritten snapshots can supply explicit `#[contime(events = [...])]` module
/// metadata. The earlier `contime_module` syntax remains supported.
///
/// # Names and aliases
/// Registrations deduplicate by name with Rust type-equality checks. Use consistent
/// event names across snapshots. Different aliases for the same type are not
/// discovered automatically. Inferred snapshots use their definition path to
/// locate metadata; use the original qualified path or explicit event metadata
/// for opaque snapshot aliases.
#[proc_macro_derive(ContimeModule, attributes(contime, contime_module))]
pub fn module(input: TokenStream) -> TokenStream {
    expansion::result(module::derive(parse_macro_input!(input as DeriveInput)))
}
/// Internal metadata collector used by module derives.
#[doc(hidden)]
#[proc_macro]
pub fn __contime_collect_module(input: TokenStream) -> TokenStream {
    expansion::result(module::collect(parse_macro_input!(input as module::State)))
}

/// Optionally implements event timestamps and identity.
///
/// Mark one time field with `#[contime(time)]`. Mark one directly stored u128
/// event-ID field with `#[contime(id)]`, or use a struct-level field selector:
/// `#[contime(id = |value| (value.client, value.sequence), map = |fields| hash(fields))]`.
/// The optional map returns u128 and requires a struct-level selector. Field
/// shorthand cannot be combined with a selector or map. Time/identity helpers
/// are independent, preserve generics, and add no payload Send/Sync bounds.
#[proc_macro_derive(ContimeEvent, attributes(contime))]
pub fn event(input: TokenStream) -> TokenStream {
    expansion::result(helpers::event(parse_macro_input!(input as DeriveInput)))
}
/// Optionally implements snapshot time, identity, registration and initialization.
///
/// Mark one directly stored u128 field with `#[contime(id)]`, or select identity
/// fields with `#[contime(id = |value| value.list_id)]`; an
/// optional `map` transforms that selection into u128. A field marker cannot be
/// combined with a struct-level selector or map. Select direct fields or
/// tuples; borrow non-Copy sources with `&value.name`. Mark the time field with
/// `#[contime(time)]`. Supply Clone and Default separately.
///
/// `#[contime(events = [Added, Removed])]` uses the snapshot selector/map for each
/// event. Singular `#[contime(event = Forwarded, id = |v| v.destination)]` permits
/// source-selector/map overrides; omitted options inherit the snapshot's settings.
/// Initialization copies corresponding source fields and defaults other state.
/// Routing never constructs snapshots. Final initialized IDs must match routing.
///
/// Generated registration and creation implement `SnapshotEvents` and
/// `SnapshotCreate`. Keep `SnapshotApply` as an ordinary impl; its default method
/// is empty. Handwritten traits remain supported for custom initialization/fanout.
/// An opaque &Self identity function remains supported without mapping; automatic
/// registration instead requires explicit identity fields. Earlier singular emitter
/// callbacks remain available with manual creation, separately from inferred events.
#[proc_macro_derive(ContimeSnapshot, attributes(contime))]
pub fn snapshot(input: TokenStream) -> TokenStream {
    expansion::result(helpers::snapshot(parse_macro_input!(input as DeriveInput)))
}
