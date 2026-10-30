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

/// Implements timestamp access and content identity for an event.
#[proc_macro_derive(ContimeEvent, attributes(contime, contime_event))]
pub fn event(input: TokenStream) -> TokenStream {
    result(event::expand(parse_macro_input!(input as DeriveInput)))
}
/// Implements identity, routing and borrowed event application for a snapshot.
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
