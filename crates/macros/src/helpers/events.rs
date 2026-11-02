//! Event registration reuses the snapshot selector without constructing snapshots.
use super::{
    creation,
    options::Options,
    selector::{mapped, Selection},
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

pub(super) fn expand(input: &DeriveInput, options: &Options) -> syn::Result<TokenStream> {
    if options.events.is_empty() {
        return Ok(TokenStream::new());
    }
    let root = crate::expansion::root();
    let name = &input.ident;
    let mut generics = input.generics.clone();
    generics.make_where_clause().predicates.push(syn::parse_quote!(Self: #root::checkpoints::Snapshot + #root::SnapshotId + 'static));
    let inferred = options.events.iter().any(|event| event.emitter().is_none());
    if inferred && options.events.iter().any(|event| event.emitter().is_some()) {
        return Err(syn::Error::new_spanned(
            input,
            "use manual SnapshotEvents for mixing custom emitter callbacks and field-based registrations",
        ));
    }
    let destination = if inferred {
        let id = options
            .id
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(input, "automatic registration requires an ID field or a struct-level id selector"))?;
        let selection = Selection::read(id)?;
        if !selection.unique() {
            return Err(syn::Error::new_spanned(id, "select each snapshot identity field once"));
        }
        Some(selection)
    } else {
        None
    };
    let mut registrations = Vec::new();
    let mut sources = Vec::new();
    for event in &options.events {
        let ty = &event.ty;
        generics.make_where_clause().predicates.push(syn::parse_quote!(
            #ty: #root::EventId<Time = <Self as #root::checkpoints::Snapshot>::Time> + 'static
        ));
        if let Some(emitter) = event.emitter() {
            if event.map.is_some() {
                return Err(syn::Error::new_spanned(emitter, "map requires a field selector, not an emitter callback"));
            }
            registrations.push(quote!(events.event::<#ty>(#emitter);));
        } else {
            let selection = match &event.id {
                Some(id) => Selection::read(id)?,
                None => destination.as_ref().unwrap().clone(),
            };
            if !selection.same_shape(destination.as_ref().unwrap()) {
                return Err(syn::Error::new_spanned(
                    ty,
                    "event and snapshot identity selectors must have the same field count and tuple shape",
                ));
            }
            let value = mapped(selection.render(quote!(__event)), event.map.as_ref().or(options.map.as_ref()));
            registrations.push(quote!(events.event::<#ty>(|__event, __emit| { __emit(#value); });));
            sources.push((ty.clone(), selection));
        }
    }
    let (ig, tg, wc) = generics.split_for_impl();
    let registration = quote! {
        impl #ig #root::SnapshotEvents for #name #tg #wc {
            fn define(events: &mut #root::EventSetBuilder<Self>) { #(#registrations)* }
        }
    };
    let initialization =
        if let Some(destination) = destination { creation::expand(input, &generics, &destination, &sources)? } else { TokenStream::new() };
    Ok(quote!(#registration #initialization))
}
