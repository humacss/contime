//! Concrete event aliases and snapshot-parameter bindings for module assembly.
use super::{metadata_parameters::Lower, metadata_paths, options::Options};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Type};
pub(super) fn expand(input: &DeriveInput, options: &Options) -> syn::Result<TokenStream> {
    let root = crate::expansion::root();
    let name = &input.ident;
    let types = format_ident!("__contime_snapshot_types_{}", name);
    let callback = format_ident!("__contime_snapshot_{}", name);
    let suffix = crate::registration::suffix(&crate::registration::key(name));
    let key = format_ident!("__contime_snapshot_key_{}_{}", name, suffix);
    let exported = format_ident!("__contime_metadata_{}", key);
    let definition = format_ident!("__ContimeParameters_{}", suffix);
    let mut aliases = Vec::new();
    let mut wrappers = Vec::new();
    let mut events = Vec::new();
    let mut lower = Lower::new(input);
    for (i, event) in options.events.iter().enumerate() {
        let Type::Path(path) = &event.ty else {
            return Err(syn::Error::new_spanned(&event.ty, "event metadata requires a named type path"));
        };
        let registration = &path.path.segments.last().unwrap().ident;
        let public_alias = format_ident!("Event{}", i);
        let source_alias = format_ident!("__Event{}_{}", suffix, i);
        let event = metadata_paths::source_type(event.ty.clone(), 2);
        let (event, range) = lower.event(event);
        let mut args = lower.arguments[range].iter().collect::<Vec<_>>();
        args.sort_by_key(|a| !matches!(a, super::metadata_parameters::Argument::Lifetime { .. }));
        let params = args.iter().map(|a| a.declaration()).collect::<Vec<_>>();
        let local = args.iter().map(|a| a.local()).collect::<Vec<_>>();
        let bindings = args.iter().map(|a| a.binding(&definition)).collect::<Vec<_>>();
        let declaration = if params.is_empty() { quote!() } else { quote!(<#(#params),*>) };
        let arguments = if local.is_empty() { quote!() } else { quote!(<#(#local),*>) };
        let binding = if bindings.is_empty() { quote!() } else { quote!(<#(#bindings),*>) };
        aliases.push(quote!(#[allow(type_alias_bounds,private_interfaces)] pub type #source_alias #declaration = #event;));
        wrappers.push(quote!(#[allow(type_alias_bounds)] pub type #public_alias #declaration = __source::#source_alias #arguments;));
        events.push(quote!(#registration=$($prefix)*::#public_alias #binding));
    }
    let declarations = &lower.declarations;
    let definitions = &lower.definitions;
    let generics = metadata_paths::source_generics(input.generics.clone(), 2);
    let (ig, tg, wc) = generics.split_for_impl();
    Ok(quote! {
        #[doc(hidden)] #[allow(non_snake_case,non_camel_case_types)]
        pub mod #types {
            #[allow(unused_imports)] use super::*;
            pub trait #definition {#(#declarations)*}
            pub mod __source {
                #[allow(unused_imports)] use super::super::*;
                impl #ig super::#definition for #name #tg #wc {#(#definitions)*}
                #(#aliases)*
            }
            #(#wrappers)*
            pub type __ContimeTime<S>=<S as #root::checkpoints::Snapshot>::Time;
        }
        #[doc(hidden)] #[macro_export] macro_rules! #exported {
            ($collector:path,[$($prefix:tt)*],[$($ancestors:ident),*],[$($state:tt)*],$registration:ident=$snapshot:path,[$($selection:tt)*])=>{
                $collector!{$($state)* #key [$($ancestors),*] $($prefix)*::__ContimeTime<$snapshot>
                    [#(#events),*] [$registration=$snapshot] [] snapshot [$($selection)*]}
            }
        }
        #[doc(hidden)] #[allow(unused_imports)] pub use #exported as #callback;
    })
}
