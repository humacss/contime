//! Derives for statically composed ConTime modules.
use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};
mod attributes;
mod event;
mod expansion;
mod module;
mod registration;
mod snapshot;
use expansion::result;

/// Implements timestamp access and an application-defined event identity.
///
/// Supply `#[contime_event(id = |event| ...)]`, returning a `u128` from a borrowed
/// event. The result is used unchanged; no hashing or namespace is added.
/// `hash = true` optionally generates `Hash` when the chosen ID function needs it.
#[proc_macro_derive(ContimeEvent, attributes(contime, contime_event))]
pub fn event(input: TokenStream) -> TokenStream {
    result(event::expand(parse_macro_input!(input as DeriveInput)))
}
/// Implements identity, routing and borrowed event application for a snapshot.
///
/// Supply `id = |key_field, ...| ...`, returning `u128`. Parameter names identify
/// the snapshot's logical key fields; each parameter borrows that field's type.
/// Routes and `snapshot_id(...)` call the same function. The derive adds no
/// hashing or namespace. Clone and Default implementations are generated unless
/// explicitly disabled. Optional `create = |snapshot_id, event| ...` supplies
/// event-based initial state; matching route key fields are filled afterward.
#[proc_macro_derive(ContimeSnapshot, attributes(contime, contime_snapshot))]
pub fn snapshot(input: TokenStream) -> TokenStream {
    result(snapshot::expand(parse_macro_input!(input as DeriveInput)))
}
/// Composes event and snapshot lanes into a static module.
#[proc_macro_derive(ContimeModule, attributes(contime, contime_module))]
pub fn module(input: TokenStream) -> TokenStream {
    result(module::derive(parse_macro_input!(input as DeriveInput)))
}
/// Internal metadata collector used by module derives.
#[doc(hidden)]
#[proc_macro]
pub fn __contime_collect_module(input: TokenStream) -> TokenStream {
    result(module::collect(parse_macro_input!(input as module::State)))
}

/// Internal definition identity resolver used by module derives.
#[doc(hidden)]
#[proc_macro]
pub fn __contime_register_module(input: TokenStream) -> TokenStream {
    result(module::register(parse_macro_input!(input as module::Registration)))
}
