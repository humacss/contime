//! Static ConTime module composition over ordinary event and snapshot traits.
use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};
mod attributes;
mod expansion;
mod module;
mod registration;
/// Assembles explicit event, snapshot and child-module lists into flat lanes.
///
/// Registrations are deduplicated by registration name, with Rust type-equality
/// checks guarding merges. A bare type uses its final path identifier as its name.
/// Use `events = [Added = Incoming]` to retain the name `Added` when the type is
/// imported as `Incoming`; the same syntax is supported in `snapshots`.
///
/// # Alias limitation
///
/// Registering the same concrete type under different names is not automatically
/// deduplicated and causes conflicting generated `From` implementations at compile
/// time. Use one consistent registration name per type across composed modules.
#[proc_macro_derive(ContimeModule, attributes(contime_module))]
pub fn module(input: TokenStream) -> TokenStream {
    expansion::result(module::derive(parse_macro_input!(input as DeriveInput)))
}
/// Internal metadata collector used by module derives.
#[doc(hidden)]
#[proc_macro]
pub fn __contime_collect_module(input: TokenStream) -> TokenStream {
    expansion::result(module::collect(parse_macro_input!(input as module::State)))
}
