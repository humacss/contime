//! Export definition-site module metadata for nested composition.
use super::metadata::{definition_path, sibling};
use crate::attributes::Options;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::DeriveInput;

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(&input.generics, "module markers currently require concrete types"));
    }
    if !matches!(&input.data,syn::Data::Struct(data) if matches!(data.fields,syn::Fields::Unit)) {
        return Err(syn::Error::new_spanned(&input, "module markers must be empty unit structs"));
    }
    let options = Options::read(&input)?;
    let root = crate::expansion::root();
    let name = &input.ident;
    let vis = &input.vis;
    let key = format_ident!("__contime_key_{}_{}", name, crate::registration::suffix(&crate::registration::key(name)));
    let exported = format_ident!("__contime_metadata_{}", key);
    let callback = format_ident!("__contime_module_{}", name);
    let types = format_ident!("__contime_types_{}", name);
    let mut aliases = Vec::new();
    let mut events = Vec::new();
    let mut snapshots = Vec::new();
    let mut deps = Vec::new();
    for (i, event) in options.events.iter().enumerate() {
        let alias = format_ident!("Event{}", i);
        let n = &event.name;
        let path = crate::helpers::module_type(&event.path);
        aliases.push(quote!(pub type #alias=#path;));
        events.push(quote!(#n=$($prefix)*::#alias));
    }
    for (i, snapshot) in options.snapshots.iter().enumerate() {
        let alias = format_ident!("Snapshot{}", i);
        let n = &snapshot.name;
        let path = crate::helpers::module_type(&snapshot.path);
        aliases.push(quote!(pub type #alias=#path;));
        let subset = if let Some(allowed) = &snapshot.subset {
            let mut entries = Vec::new();
            for (j, event) in allowed.iter().enumerate() {
                let alias = format_ident!("SnapshotEvent{}_{}", i, j);
                let event_name = &event.name;
                let path = crate::helpers::module_type(&event.path);
                aliases.push(quote!(pub type #alias=#path;));
                entries.push(quote!(#event_name=$($prefix)*::#alias));
            }
            quote!(only [#(#entries),*])
        } else {
            quote!()
        };
        if options.infer_events {
            let metadata = format_ident!("SnapshotMetadata{}", i);
            let metadata_types = format_ident!("SnapshotTypes{}", i);
            let callback_path = definition_path(&sibling(&snapshot.path, "__contime_snapshot_"));
            let types_path = definition_path(&sibling(&snapshot.path, "__contime_snapshot_types_"));
            aliases.push(quote!(pub use #callback_path as #metadata; pub use #types_path as #metadata_types;));
            deps.push(quote!(($($prefix)*::#metadata,$($prefix)*::#metadata_types,[$($ancestors,)* #key]; #n=$($prefix)*::#alias #subset)));
        } else {
            snapshots.push(quote!(#n=$($prefix)*::#alias #subset));
        }
    }
    for (i, path) in options.modules.iter().enumerate() {
        let alias = format_ident!("Module{}", i);
        let type_alias = format_ident!("Dependency{}", i);
        let module_type = format_ident!("DependencyType{}", i);
        let callback = definition_path(&sibling(path, "__contime_module_"));
        let metadata_types = definition_path(&sibling(path, "__contime_types_"));
        let path = definition_path(path);
        aliases.push(quote!(pub use #callback as #alias; pub use #metadata_types as #type_alias; pub type #module_type=#path;));
        deps.push(quote!(($($prefix)*::#alias,$($prefix)*::#type_alias,[$($ancestors,)* #key])));
    }
    let time = if let Some(time) = &options.time_type {
        quote!(#time)
    } else if !options.snapshots.is_empty() {
        quote!(<Snapshot0 as #root::checkpoints::Snapshot>::Time)
    } else if !options.modules.is_empty() {
        quote!(<DependencyType0 as #root::module::Module>::Time)
    } else {
        return Err(syn::Error::new_spanned(
            name,
            "cannot infer time from an empty module; provide snapshots, child modules, or explicit time",
        ));
    };
    Ok(quote! {
        #[doc(hidden)] #[allow(non_snake_case,non_camel_case_types)]
        pub mod #types {use super::*; pub type __ContimeTime=#time; #(#aliases)*}
        #[doc(hidden)] #[macro_export] macro_rules! #exported {
            ($collector:path,[$($prefix:tt)*],[$($ancestors:ident),*],[$($state:tt)*])=>{
                $collector!{$($state)* #key [$($ancestors),*] $($prefix)*::__ContimeTime [#(#events),*] [#(#snapshots),*] [#(#deps),*]}
            }
        }
        #[doc(hidden)] pub use #exported as #callback;
        #callback!(#root::__contime_collect_module,[#types],[],[#vis #name;#types::__ContimeTime; [] [] [] []]);
    })
}
