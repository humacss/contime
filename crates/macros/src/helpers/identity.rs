//! Parse optional ID functions and delegate without selecting a hashing policy.
use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Expr};
pub(super) fn event(input: &DeriveInput, function: &Expr) -> TokenStream {
    event_body(
        input,
        quote! {
            let function: fn(&Self) -> u128 = #function;
            function(self)
        },
    )
}
pub(super) fn snapshot(input: &DeriveInput, function: &Expr) -> TokenStream {
    snapshot_body(
        input,
        quote! {
            let function: fn(&Self) -> u128 = #function;
            function(self)
        },
    )
}
pub(super) fn event_body(input: &DeriveInput, body: TokenStream) -> TokenStream {
    let root = crate::expansion::root();
    let name = &input.ident;
    let mut generics = input.generics.clone();
    generics.make_where_clause().predicates.push(syn::parse_quote!(Self: #root::checkpoints::Event));
    let (ig, tg, wc) = generics.split_for_impl();
    quote! { impl #ig #root::EventId for #name #tg #wc { fn event_id(&self) -> u128 { #body } } }
}
pub(super) fn snapshot_body(input: &DeriveInput, body: TokenStream) -> TokenStream {
    let root = crate::expansion::root();
    let name = &input.ident;
    let (ig, tg, wc) = input.generics.split_for_impl();
    quote! { impl #ig #root::SnapshotId for #name #tg #wc { fn snapshot_id(&self) -> u128 { #body } } }
}
