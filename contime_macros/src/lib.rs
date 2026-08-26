//! Hidden proc-macro helpers for `contime`.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parse_macro_input, Block, DeriveInput, Error, Expr, Fields, Ident, Path, Result, Token, Type};

#[proc_macro]
pub fn __lanes_merge(input: TokenStream) -> TokenStream {
    match expand_lanes(parse_macro_input!(input as LanesManifest)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro]
pub fn lanes(input: TokenStream) -> TokenStream {
    match expand_lanes(parse_macro_input!(input as NewLanesManifest)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(ContimeEvent, attributes(contime_event))]
pub fn derive_contime_event(input: TokenStream) -> TokenStream {
    match expand_contime_event(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(ContimeSnapshot, attributes(contime_snapshot))]
pub fn derive_contime_snapshot(input: TokenStream) -> TokenStream {
    match expand_contime_snapshot(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_lanes(input: impl Into<LanesManifest>) -> Result<TokenStream2> {
    let input = input.into();
    let snapshots = dedupe_snapshots(&input.snapshots)?;
    let routes = merge_routes(&input.routes)?;
    let markers = dedupe_snapshots(&input.markers)?;
    validate_route_targets(&snapshots, &routes)?;
    validate_input_variants(&routes, &markers)?;
    let time_type = input.time_type;

    let snapshot_variants = snapshots
        .iter()
        .map(|snapshot| {
            let variant = &snapshot.variant;
            let ty = &snapshot.path;
            quote! { #variant(#ty), }
        })
        .collect::<Vec<_>>();

    let snapshot_id_arms = snapshots
        .iter()
        .map(|snapshot| {
            let variant = &snapshot.variant;
            let ty = &snapshot.path;
            quote! { Self::#variant(s) => <#ty as ::contime::Snapshot>::id(s), }
        })
        .collect::<Vec<_>>();

    let snapshot_time_arms = snapshots
        .iter()
        .map(|snapshot| {
            let variant = &snapshot.variant;
            let ty = &snapshot.path;
            quote! { Self::#variant(s) => <#ty as ::contime::Snapshot>::time(s), }
        })
        .collect::<Vec<_>>();

    let snapshot_set_time_arms = snapshots
        .iter()
        .map(|snapshot| {
            let variant = &snapshot.variant;
            let ty = &snapshot.path;
            quote! { Self::#variant(s) => <#ty as ::contime::Snapshot>::set_time(s, time), }
        })
        .collect::<Vec<_>>();

    let snapshot_size_arms = snapshots
        .iter()
        .map(|snapshot| {
            let variant = &snapshot.variant;
            let ty = &snapshot.path;
            quote! { Self::#variant(s) => <#ty as ::contime::Snapshot>::conservative_size(s), }
        })
        .collect::<Vec<_>>();

    let snapshot_compact_arms = snapshots
        .iter()
        .map(|snapshot| {
            let variant = &snapshot.variant;
            let ty = &snapshot.path;
            quote! { Self::#variant(s) => <#ty as ::contime::Snapshot>::compact_before(s, time), }
        })
        .collect::<Vec<_>>();

    let snapshot_from_impls = snapshots
        .iter()
        .map(|snapshot| {
            let variant = &snapshot.variant;
            let ty = &snapshot.path;
            quote! {
                impl From<#ty> for SnapshotLanes {
                    fn from(snapshot: #ty) -> Self {
                        Self::#variant(snapshot)
                    }
                }

                impl From<SnapshotLanes> for #ty {
                    fn from(lane: SnapshotLanes) -> Self {
                        match lane {
                            SnapshotLanes::#variant(snapshot) => snapshot,
                            other => panic!(
                                "cannot convert snapshot lane {:?} into {}",
                                other,
                                stringify!(#ty)
                            ),
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    let event_variants = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            quote! { #key(#event_ty), }
        })
        .collect::<Vec<_>>();

    let marker_variants = markers
        .iter()
        .map(|marker| {
            let variant = &marker.variant;
            let ty = &marker.path;
            quote! { #variant(#ty), }
        })
        .collect::<Vec<_>>();

    let event_id_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            quote! { Self::#key(e) => <#event_ty as ::contime::Input>::id(e), }
        })
        .collect::<Vec<_>>();

    let event_time_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            quote! { Self::#key(e) => <#event_ty as ::contime::Input>::time(e), }
        })
        .collect::<Vec<_>>();

    let event_size_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            quote! { Self::#key(e) => <#event_ty as ::contime::Input>::conservative_size(e), }
        })
        .collect::<Vec<_>>();

    let marker_id_arms = markers
        .iter()
        .map(|marker| {
            let variant = &marker.variant;
            let ty = &marker.path;
            quote! { Self::#variant(marker) => <#ty as ::contime::Input>::id(marker), }
        })
        .collect::<Vec<_>>();

    let marker_time_arms = markers
        .iter()
        .map(|marker| {
            let variant = &marker.variant;
            let ty = &marker.path;
            quote! { Self::#variant(marker) => <#ty as ::contime::Input>::time(marker), }
        })
        .collect::<Vec<_>>();

    let marker_size_arms = markers
        .iter()
        .map(|marker| {
            let variant = &marker.variant;
            let ty = &marker.path;
            quote! { Self::#variant(marker) => <#ty as ::contime::Input>::conservative_size(marker), }
        })
        .collect::<Vec<_>>();

    let mut apply_bounds = Vec::new();
    for route in &routes {
        let event_ty = &route.event_ty;
        for target in &route.targets {
            let target_ty = &target.path;
            apply_bounds.push(quote! {
                #event_ty: ::contime::SnapshotEvent<#target_ty>
            });
            apply_bounds.push(quote! {
                #target_ty: ::core::default::Default
            });
            apply_bounds.push(quote! {
                #target_ty: ::contime::ApplyEvents<<#target_ty as ::contime::Snapshot>::Input>
            });
            apply_bounds.push(quote! {
                <#target_ty as ::contime::Snapshot>::Input: From<#event_ty>
            });
        }
    }
    let marker_route_bounds = markers.iter().map(|marker| {
        let ty = &marker.path;
        quote! { #ty: ::contime::InputRoute }
    });

    let apply_snapshot_arms = snapshots
        .iter()
        .map(|snapshot| {
            let snapshot_variant = &snapshot.variant;
            let snapshot_ty = &snapshot.path;
            let snapshot_key = normalized_path_key(snapshot_ty);
            let route_pushes = routes.iter().filter_map(|route| {
                if route.targets.iter().any(|target| normalized_path_key(&target.path) == snapshot_key) {
                    let key = &route.key;
                    Some(quote! {
                        for event in batch.inputs.iter().copied() {
                            if let InputLanes::#key(event) = event {
                                bucket.push(event.clone().into());
                            }
                        }
                    })
                } else {
                    None
                }
            });
            quote! {
                SnapshotLanes::#snapshot_variant(snapshot) => {
                    let mut bucket = Vec::new();
                    #( #route_pushes )*
                    if !bucket.is_empty() {
                        let bucket = bucket.iter().collect::<Vec<_>>();
                        <#snapshot_ty as ::contime::ApplyEvents<<#snapshot_ty as ::contime::Snapshot>::Input>>::apply_events(
                            snapshot,
                            ::contime::ApplyBatch {
                                snapshot_id: batch.snapshot_id,
                                time: batch.time.clone(),
                                history_input_count,
                                events: &bucket,
                            },
                        );
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    let event_snapshot_ids_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            let targets = route.targets.iter().map(|target| {
                let target_ty = &target.path;
                quote! {
                    visit(<#event_ty as ::contime::SnapshotEvent<#target_ty>>::snapshot_id(e));
                }
            });
            quote! {
                Self::#key(e) => {
                    #( #targets )*
                }
            }
        })
        .collect::<Vec<_>>();

    let marker_snapshot_ids_arms = markers
        .iter()
        .map(|marker| {
            let variant = &marker.variant;
            let ty = &marker.path;
            quote! {
                Self::#variant(marker) => <#ty as ::contime::InputRoute>::visit_snapshot_ids(marker, visit),
            }
        })
        .collect::<Vec<_>>();

    let materialize_event_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            let candidates = route.targets.iter().map(|target| {
                let target_variant = &target.variant;
                let target_ty = &target.path;
                quote! {
                    if <#event_ty as ::contime::SnapshotEvent<#target_ty>>::snapshot_id(event) == snapshot_id {
                        if materialized.is_some() {
                            panic!("snapshot id {} maps one event to multiple snapshot lanes", snapshot_id);
                        }
                        let mut snapshot = <#target_ty as ::core::default::Default>::default();
                        <#event_ty as ::contime::SnapshotEvent<#target_ty>>::set_snapshot_identity(event, &mut snapshot);
                        assert_eq!(
                            <#target_ty as ::contime::Snapshot>::id(&snapshot),
                            snapshot_id,
                            "set_snapshot_identity produced the wrong snapshot id",
                        );
                        materialized = Some(SnapshotLanes::#target_variant(snapshot));
                    }
                }
            });
            quote! {
                InputLanes::#key(event) => {
                    let mut materialized = None;
                    #( #candidates )*
                    materialized
                }
            }
        })
        .collect::<Vec<_>>();

    let materialize_marker_arms = markers.iter().map(|marker| {
        let variant = &marker.variant;
        quote! { InputLanes::#variant(_) => None, }
    });

    let snapshot_lane_index_arms = snapshots
        .iter()
        .enumerate()
        .map(|(index, snapshot)| {
            let variant = &snapshot.variant;
            quote! { Self::#variant(_) => #index, }
        })
        .collect::<Vec<_>>();

    let input_lane_index_event_arms = routes
        .iter()
        .map(|route| {
            let event_variant = &route.key;
            let event_ty = &route.event_ty;
            let candidates = route.targets.iter().map(|target| {
                let target_ty = &target.path;
                let target_key = normalized_path_key(target_ty);
                let target_index = snapshots
                    .iter()
                    .position(|snapshot| normalized_path_key(&snapshot.path) == target_key)
                    .expect("validated route target must exist");
                quote! {
                    if <#event_ty as ::contime::SnapshotEvent<#target_ty>>::snapshot_id(event) == snapshot_id {
                        if lane_index.replace(#target_index).is_some() {
                            panic!("snapshot id {} maps one event to multiple snapshot lanes", snapshot_id);
                        }
                    }
                }
            });
            quote! {
                InputLanes::#event_variant(event) => {
                    let mut lane_index = None;
                    #( #candidates )*
                    lane_index
                }
            }
        })
        .collect::<Vec<_>>();

    let input_lane_index_marker_arms = markers.iter().map(|marker| {
        let variant = &marker.variant;
        quote! { InputLanes::#variant(_) => None, }
    });

    let event_kind_arms = routes.iter().map(|route| {
        let key = &route.key;
        quote! { Self::#key(_) => true, }
    });
    let marker_kind_arms = markers.iter().map(|marker| {
        let variant = &marker.variant;
        quote! { Self::#variant(_) => false, }
    });

    let event_from_impls = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            quote! {
                impl From<#event_ty> for InputLanes {
                    fn from(event: #event_ty) -> Self {
                        Self::#key(event)
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    let marker_from_impls = markers
        .iter()
        .map(|marker| {
            let variant = &marker.variant;
            let ty = &marker.path;
            quote! {
                impl From<#ty> for InputLanes {
                    fn from(marker: #ty) -> Self {
                        Self::#variant(marker)
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    let modname = input.modname;
    let context_ty = input.context.map(|context| quote! { #context }).unwrap_or_else(|| quote! { () });

    Ok(quote! {
        mod #modname {
            use super::*;

            #[derive(Clone, Debug, PartialEq, Eq)]
            pub enum SnapshotLanes {
                #( #snapshot_variants )*
            }

            impl ::contime::SnapshotLanes for SnapshotLanes {
                fn materialize(snapshot_id: u128, input: &Self::Input) -> Option<Self> {
                    match input {
                        #( #materialize_event_arms )*
                        #( #materialize_marker_arms )*
                    }
                }

                fn lane_index(&self) -> usize {
                    match self {
                        #( #snapshot_lane_index_arms )*
                    }
                }

                fn input_lane_index(snapshot_id: u128, input: &Self::Input) -> Option<usize> {
                    match input {
                        #( #input_lane_index_event_arms )*
                        #( #input_lane_index_marker_arms )*
                    }
                }
            }

            impl ::contime::Snapshot for SnapshotLanes {
                type Time = #time_type;
                type Input = InputLanes;

                fn id(&self) -> u128 {
                    match self {
                        #( #snapshot_id_arms )*
                    }
                }

                fn time(&self) -> Self::Time {
                    match self {
                        #( #snapshot_time_arms )*
                    }
                }

                fn set_time(&mut self, time: Self::Time) {
                    match self {
                        #( #snapshot_set_time_arms )*
                    }
                }

                fn conservative_size(&self) -> u64 {
                    match self {
                        #( #snapshot_size_arms )*
                    }
                }

                fn compact_before(&mut self, time: Self::Time) {
                    match self {
                        #( #snapshot_compact_arms )*
                    }
                }

            }

            #( #snapshot_from_impls )*

            #[derive(Debug, Clone, Eq, PartialEq)]
            pub enum InputLanes {
                #( #event_variants )*
                #( #marker_variants )*
            }

            impl ::contime::Input for InputLanes {
                type Time = #time_type;

                fn id(&self) -> u128 {
                    match self {
                        #( #event_id_arms )*
                        #( #marker_id_arms )*
                    }
                }

                fn time(&self) -> Self::Time {
                    match self {
                        #( #event_time_arms )*
                        #( #marker_time_arms )*
                    }
                }

                fn conservative_size(&self) -> u64 {
                    match self {
                        #( #event_size_arms )*
                        #( #marker_size_arms )*
                    }
                }
            }

            impl ::contime::InputLanes<SnapshotLanes> for InputLanes
            where
                #( #apply_bounds, )*
                #( #marker_route_bounds, )*
            {
                fn visit_snapshot_ids<F>(&self, visit: &mut F)
                where
                    F: FnMut(u128),
                {
                    match self {
                        #( #event_snapshot_ids_arms )*
                        #( #marker_snapshot_ids_arms )*
                    }
                }

                fn is_event(&self) -> bool {
                    match self {
                        #( #event_kind_arms )*
                        #( #marker_kind_arms )*
                    }
                }

                fn apply_events(
                    snapshot: &mut SnapshotLanes,
                    batch: ::contime::InputBatch<'_, Self>,
                    history_input_count: u64,
                ) {
                    match snapshot {
                        #( #apply_snapshot_arms )*
                    }
                }
            }

            #( #event_from_impls )*
            #( #marker_from_impls )*

            pub type Contime = ::contime::Contime<SnapshotLanes, InputLanes, #context_ty>;
        }
    })
}

fn expand_contime_event(input: DeriveInput) -> Result<TokenStream2> {
    let name = input.ident;
    let attr = input
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("contime_event"))
        .ok_or_else(|| Error::new(name.span(), "missing `#[contime_event(...)]` attribute"))?;
    let config = attr.parse_args::<EventDeriveConfig>()?;
    let id = config.id.ok_or_else(|| Error::new(attr.span(), "`contime_event` requires `id = ...`"))?;
    let time = config.time.ok_or_else(|| Error::new(attr.span(), "`contime_event` requires `time = ...`"))?;
    let time_type = config.time_type.unwrap_or_else(|| syn::parse_quote!(i64));
    let bytes = config.bytes.ok_or_else(|| Error::new(attr.span(), "`contime_event` requires `bytes = ...`"))?;

    Ok(quote! {
        impl ::contime::Input for #name {
            type Time = #time_type;

            fn id(&self) -> u128 {
                #id
            }

            fn time(&self) -> Self::Time {
                #time
            }

            fn conservative_size(&self) -> u64 {
                #bytes
            }
        }

        impl ::contime::Event for #name {}
    })
}

fn expand_contime_snapshot(input: DeriveInput) -> Result<TokenStream2> {
    let name = input.ident.clone();
    let generics = input.generics.clone();
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let type_turbofish = type_generics.as_turbofish();
    let attr = input
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("contime_snapshot"))
        .ok_or_else(|| Error::new(name.span(), "missing `#[contime_snapshot(...)]` attribute"))?;
    match input.data {
        syn::Data::Struct(data) => match data.fields {
            Fields::Named(_) => {}
            other => {
                return Err(Error::new(other.span(), "`ContimeSnapshot` currently requires a struct with named fields"));
            }
        },
        other => {
            let _ = other;
            return Err(Error::new(name.span(), "`ContimeSnapshot` can only be derived for structs"));
        }
    }

    let config = attr.parse_args::<SnapshotDeriveConfig>()?;
    let events = config.events.ok_or_else(|| Error::new(attr.span(), "`contime_snapshot` requires `events = [...]`"))?;
    if events.is_empty() {
        return Err(Error::new(attr.span(), "`contime_snapshot` requires at least one event"));
    }
    let ids = config.ids.ok_or_else(|| Error::new(attr.span(), "`contime_snapshot` requires `id = [...]`"))?;
    if ids.len() != 1 {
        return Err(Error::new(attr.span(), "`ContimeSnapshot` currently supports exactly one id field"));
    }
    let id = ids.first().expect("checked len").clone();
    let time = config.time.unwrap_or_else(|| syn::parse_quote!(self.time.clone()));
    let time_type = config.time_type.unwrap_or_else(|| syn::parse_quote!(i64));
    let bytes = config.bytes.ok_or_else(|| Error::new(attr.span(), "`contime_snapshot` requires `bytes = ...`"))?;
    let compact = config.compact.map(|compact| {
        quote! {
            fn compact_before(&mut self, time: Self::Time) {
                let time = time;
                #compact
            }
        }
    });
    let apply = config.apply.ok_or_else(|| Error::new(attr.span(), "`contime_snapshot` requires `apply = { ... }`"))?;

    let event_enum = Ident::new(&format!("{name}Event"), name.span());
    let snapshot_lanes_macro = Ident::new(&format!("__ao_{name}_snapshot_lanes"), name.span());
    let event_lanes_macro = Ident::new(&format!("__ao_{name}_event_lanes"), name.span());
    let snapshot_fragment_macro = Ident::new(&format!("__ao_snapshot_fragment_{name}"), name.span());
    let event_variants = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event);
            variant.map(|variant| quote! { #variant(#event), })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_id_arms = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            Ok(quote! { Self::#variant(event) => <#event as ::contime::Input>::id(event), })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_time_arms = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            Ok(quote! { Self::#variant(event) => <#event as ::contime::Input>::time(event), })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_size_arms = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            Ok(quote! { Self::#variant(event) => <#event as ::contime::Input>::conservative_size(event), })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_snapshot_id_arms = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            Ok(quote! {
                Self::#variant(event) => <#event as ::contime::SnapshotEvent<#name #type_generics>>::snapshot_id(event),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_set_snapshot_identity_arms = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            Ok(quote! {
                Self::#variant(event) => <#event as ::contime::SnapshotEvent<#name #type_generics>>::set_snapshot_identity(event, snapshot),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_from_impls = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            Ok(quote! {
                impl #impl_generics From<#event> for #event_enum #type_generics #where_clause {
                    fn from(event: #event) -> Self {
                        Self::#variant(event)
                    }
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_snapshot_impls = events
        .iter()
        .map(|event| {
            let id = &id;
            Ok(quote! {
                impl #impl_generics ::contime::SnapshotEvent<#name #type_generics> for #event #where_clause {
                    fn snapshot_id(&self) -> u128 {
                        self.#id
                    }

                    fn set_snapshot_identity(&self, snapshot: &mut #name #type_generics) {
                        snapshot.#id = self.#id;
                    }
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let fragment_event_routes = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            let event_key = syn::LitStr::new(&variant.to_string(), variant.span());
            Ok(quote! {
                #variant(#event) [key = #event_key] => #event_enum #type_turbofish => [#name #type_turbofish],
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum #event_enum #generics {
            #( #event_variants )*
        }

        impl #impl_generics ::contime::Input for #event_enum #type_generics #where_clause {
            type Time = #time_type;

            fn id(&self) -> u128 {
                match self {
                    #( #event_id_arms )*
                }
            }

            fn time(&self) -> Self::Time {
                match self {
                    #( #event_time_arms )*
                }
            }

            fn conservative_size(&self) -> u64 {
                match self {
                    #( #event_size_arms )*
                }
            }
        }

        impl #impl_generics ::contime::Event for #event_enum #type_generics #where_clause {}

        impl #impl_generics ::contime::SnapshotEvent<#name #type_generics> for #event_enum #type_generics #where_clause {
            fn snapshot_id(&self) -> u128 {
                match self {
                    #( #event_snapshot_id_arms )*
                }
            }

            fn set_snapshot_identity(&self, snapshot: &mut #name #type_generics) {
                match self {
                    #( #event_set_snapshot_identity_arms )*
                }
            }
        }

        #( #event_from_impls )*
        #( #event_snapshot_impls )*
        impl #impl_generics ::contime::Snapshot for #name #type_generics #where_clause {
            type Time = #time_type;
            type Input = #event_enum #type_generics;

            fn id(&self) -> u128 {
                self.#id
            }

            fn time(&self) -> Self::Time {
                #time
            }

            fn set_time(&mut self, time: Self::Time) {
                self.time = time;
            }

            fn conservative_size(&self) -> u64 {
                #bytes
            }

            #compact

        }

        impl #impl_generics ::contime::ApplyEvents<#event_enum #type_generics> for #name #type_generics #where_clause {
            fn apply_events(&mut self, batch: ::contime::ApplyBatch<'_, #event_enum #type_generics>) {
                let batch = batch;
                #apply
            }
        }

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #snapshot_lanes_macro {
            (
                @ao_collect_enum
                enum $name:ident
                generics { $($generics:tt)* }
                vis { $vis:vis }
                attrs { $($attrs:tt)* }
                variants { $($variants:tt)* }
                rest [ $next:path $(, $rest:path)* $(,)? ]
            ) => {
                $next! {
                    @ao_collect_enum
                    enum $name
                    generics { $($generics)* }
                    vis { $vis }
                    attrs { $($attrs)* }
                    variants {
                        $($variants)*
                        #name(#name #type_turbofish),
                    }
                    rest [ $($rest),* ]
                }
            };
        }

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #event_lanes_macro {
            (
                @ao_collect_enum
                enum $name:ident
                generics { $($generics:tt)* }
                vis { $vis:vis }
                attrs { $($attrs:tt)* }
                variants { $($variants:tt)* }
                rest [ $next:path $(, $rest:path)* $(,)? ]
            ) => {
                $next! {
                    @ao_collect_enum
                    enum $name
                    generics { $($generics)* }
                    vis { $vis }
                    attrs { $($attrs)* }
                    variants {
                        $($variants)*
                        #name(#event_enum #type_turbofish),
                    }
                    rest [ $($rest),* ]
                }
            };
        }

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #snapshot_fragment_macro {
            (
                @append
                snapshots { $($snapshots:tt)* }
                event_routes { $($event_routes:tt)* }
                fragments [ $next:path $(, $rest:path)* $(,)? ]
            ) => {
                $next! {
                    @append
                    snapshots {
                        $($snapshots)*
                        #name #type_turbofish,
                    }
                    event_routes {
                        $($event_routes)*
                        #( #fragment_event_routes )*
                    }
                    fragments [ $($rest),* ]
                }
            };
        }
    })
}

fn dedupe_snapshots(snapshots: &[Path]) -> Result<Vec<SnapshotSpec>> {
    let mut by_type = BTreeMap::new();
    let mut by_variant = BTreeMap::new();

    for path in snapshots {
        let type_key = normalized_path_key(path);
        if by_type.contains_key(&type_key) {
            continue;
        }
        let variant = trailing_ident(path)?;
        let variant_key = variant.to_string();
        if let Some(existing) = by_variant.get(&variant_key) {
            if existing != &type_key {
                return Err(Error::new(
                    variant.span(),
                    format!("snapshot variant `{variant_key}` would refer to multiple snapshot types: `{existing}` and `{type_key}`"),
                ));
            }
        }
        by_variant.insert(variant_key, type_key.clone());
        by_type.insert(type_key, SnapshotSpec { path: path.clone(), variant });
    }

    Ok(by_type.into_values().collect())
}

fn merge_routes(routes: &[RouteEntry]) -> Result<Vec<RouteSpec>> {
    let mut merged = BTreeMap::<String, RouteSpec>::new();
    let mut event_to_key = BTreeMap::<String, String>::new();

    for route in routes {
        let key_name = route.key.to_string();
        let event_key = normalized_type_key(&route.event_ty);

        if let Some(existing_key) = event_to_key.get(&event_key) {
            if existing_key != &key_name {
                return Err(Error::new(
                    route.key.span(),
                    format!("event type `{event_key}` is routed under multiple keys: `{existing_key}` and `{key_name}`"),
                ));
            }
        } else {
            event_to_key.insert(event_key.clone(), key_name.clone());
        }

        let entry = merged.entry(key_name.clone()).or_insert_with(|| RouteSpec {
            key: route.key.clone(),
            event_ty: route.event_ty.clone(),
            event_key: event_key.clone(),
            targets: Vec::new(),
        });

        if entry.event_key != event_key {
            return Err(Error::new(
                route.key.span(),
                format!("route key `{key_name}` uses conflicting event types: `{}` and `{event_key}`", entry.event_key),
            ));
        }

        let mut seen = entry.targets.iter().map(|target| normalized_path_key(&target.path)).collect::<BTreeSet<_>>();

        for target in &route.targets {
            let target_key = normalized_path_key(target);
            if seen.insert(target_key) {
                entry.targets.push(SnapshotSpec { path: target.clone(), variant: trailing_ident(target)? });
            }
        }
    }

    if merged.is_empty() {
        return Err(Error::new(Span::call_site(), "contime::lanes! requires at least one route across the listed fragments"));
    }

    Ok(merged.into_values().collect())
}

fn validate_route_targets(snapshots: &[SnapshotSpec], routes: &[RouteSpec]) -> Result<()> {
    let known = snapshots.iter().map(|snapshot| normalized_path_key(&snapshot.path)).collect::<BTreeSet<_>>();

    for route in routes {
        for target in &route.targets {
            let target_key = normalized_path_key(&target.path);
            if !known.contains(&target_key) {
                return Err(Error::new(
                    target.path.span(),
                    format!("route target `{target_key}` is not listed in the assembled snapshots"),
                ));
            }
        }
    }

    Ok(())
}

fn validate_input_variants(routes: &[RouteSpec], markers: &[SnapshotSpec]) -> Result<()> {
    let event_variants = routes.iter().map(|route| route.key.to_string()).collect::<BTreeSet<_>>();
    for marker in markers {
        let marker_variant = marker.variant.to_string();
        if event_variants.contains(&marker_variant) {
            return Err(Error::new(
                marker.variant.span(),
                format!("input variant `{marker_variant}` cannot be both an event and a plain marker"),
            ));
        }
    }
    Ok(())
}

fn trailing_ident(path: &Path) -> Result<Ident> {
    path.segments.last().map(|segment| segment.ident.clone()).ok_or_else(|| Error::new(path.span(), "expected a named path"))
}

fn normalized_path_key(path: &Path) -> String {
    path.to_token_stream().to_string()
}

fn normalized_type_key(ty: &Type) -> String {
    ty.to_token_stream().to_string()
}

struct SnapshotSpec {
    path: Path,
    variant: Ident,
}

struct RouteSpec {
    key: Ident,
    event_ty: Type,
    event_key: String,
    targets: Vec<SnapshotSpec>,
}

struct LanesManifest {
    modname: Ident,
    context: Option<Type>,
    time_type: Type,
    snapshots: Vec<Path>,
    markers: Vec<Path>,
    routes: Vec<RouteEntry>,
}

struct NewLanesManifest {
    modname: Ident,
    context: Option<Type>,
    time_type: Type,
    snapshots: Vec<Path>,
    markers: Vec<Path>,
    routes: Vec<RouteEntry>,
}

struct EventDeriveConfig {
    id: Option<Expr>,
    time: Option<Expr>,
    time_type: Option<Type>,
    bytes: Option<Expr>,
}

struct SnapshotDeriveConfig {
    events: Option<Vec<Path>>,
    ids: Option<Vec<Ident>>,
    time: Option<Expr>,
    time_type: Option<Type>,
    bytes: Option<Expr>,
    compact: Option<Block>,
    apply: Option<Block>,
}

impl Parse for LanesManifest {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse::<Token![mod]>()?;
        let modname = input.parse::<Ident>()?;
        input.parse::<Token![;]>()?;
        let snapshots_label = input.parse::<Ident>()?;
        if snapshots_label != "snapshots" {
            return Err(Error::new(snapshots_label.span(), "expected `snapshots`"));
        }
        let snapshots_content;
        syn::braced!(snapshots_content in input);
        let snapshots = Punctuated::<Path, Token![,]>::parse_terminated(&snapshots_content)?.into_iter().collect::<Vec<_>>();

        let routes_label = input.parse::<Ident>()?;
        if routes_label != "routes" {
            return Err(Error::new(routes_label.span(), "expected `routes`"));
        }
        let routes_content;
        syn::braced!(routes_content in input);
        let routes = Punctuated::<RouteEntry, Token![,]>::parse_terminated(&routes_content)?.into_iter().collect::<Vec<_>>();

        Ok(Self { modname, context: None, time_type: syn::parse_quote!(i64), snapshots, markers: Vec::new(), routes })
    }
}

impl Parse for NewLanesManifest {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse::<Token![mod]>()?;
        let modname = input.parse::<Ident>()?;
        input.parse::<Token![;]>()?;

        let mut context = None;
        if input.peek(Ident) {
            let fork = input.fork();
            let label = fork.parse::<Ident>()?;
            if label == "context" {
                input.parse::<Ident>()?;
                context = Some(input.parse::<Type>()?);
                input.parse::<Token![;]>()?;
            }
        }

        let time_type = if input.peek(Ident) {
            let fork = input.fork();
            let label = fork.parse::<Ident>()?;
            if label == "time" {
                input.parse::<Ident>()?;
                let time_type = input.parse::<Type>()?;
                input.parse::<Token![;]>()?;
                time_type
            } else {
                syn::parse_quote!(i64)
            }
        } else {
            syn::parse_quote!(i64)
        };

        let snapshots_label = input.parse::<Ident>()?;
        if snapshots_label != "snapshots" {
            return Err(Error::new(snapshots_label.span(), "expected `snapshots`"));
        }
        let snapshots_content;
        syn::bracketed!(snapshots_content in input);
        let snapshots = Punctuated::<Path, Token![,]>::parse_terminated(&snapshots_content)?.into_iter().collect::<Vec<_>>();
        input.parse::<Token![;]>()?;

        let markers = if input.peek(Ident) {
            let fork = input.fork();
            let label = fork.parse::<Ident>()?;
            if label == "markers" {
                input.parse::<Ident>()?;
                let markers_content;
                syn::bracketed!(markers_content in input);
                let markers = Punctuated::<Path, Token![,]>::parse_terminated(&markers_content)?.into_iter().collect::<Vec<_>>();
                input.parse::<Token![;]>()?;
                markers
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let routes_label = input.parse::<Ident>()?;
        if routes_label != "routes" {
            return Err(Error::new(routes_label.span(), "expected `routes`"));
        }
        let routes_content;
        syn::bracketed!(routes_content in input);
        let routes = Punctuated::<NewRouteEntry, Token![,]>::parse_terminated(&routes_content)?
            .into_iter()
            .map(RouteEntry::from)
            .collect::<Vec<_>>();
        input.parse::<Token![;]>()?;

        Ok(Self { modname, context, time_type, snapshots, markers, routes })
    }
}

impl From<NewLanesManifest> for LanesManifest {
    fn from(value: NewLanesManifest) -> Self {
        Self {
            modname: value.modname,
            context: value.context,
            time_type: value.time_type,
            snapshots: value.snapshots,
            markers: value.markers,
            routes: value.routes,
        }
    }
}

struct RouteEntry {
    key: Ident,
    event_ty: Type,
    targets: Vec<Path>,
}

struct NewRouteEntry {
    event_ty: Path,
    targets: Vec<Path>,
}

impl Parse for RouteEntry {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let key = input.parse::<Ident>()?;
        let event_content;
        syn::parenthesized!(event_content in input);
        let event_ty = event_content.parse::<Type>()?;
        input.parse::<Token![=>]>()?;
        let targets_content;
        syn::bracketed!(targets_content in input);
        let targets = Punctuated::<Path, Token![,]>::parse_terminated(&targets_content)?.into_iter().collect::<Vec<_>>();
        Ok(Self { key, event_ty, targets })
    }
}

impl Parse for NewRouteEntry {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let event_ty = input.parse::<Path>()?;
        input.parse::<Token![=>]>()?;
        let targets_content;
        syn::bracketed!(targets_content in input);
        let targets = Punctuated::<Path, Token![,]>::parse_terminated(&targets_content)?.into_iter().collect::<Vec<_>>();
        Ok(Self { event_ty, targets })
    }
}

impl From<NewRouteEntry> for RouteEntry {
    fn from(value: NewRouteEntry) -> Self {
        let key = trailing_ident(&value.event_ty).expect("parsed path has a trailing ident");
        let event_ty = Type::Path(syn::TypePath { qself: None, path: value.event_ty });
        Self { key, event_ty, targets: value.targets }
    }
}

impl Parse for EventDeriveConfig {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut config = Self { id: None, time: None, time_type: None, bytes: None };
        while !input.is_empty() {
            let key = input.parse::<Ident>()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "id" => config.id = Some(input.parse::<Expr>()?),
                "time" => config.time = Some(input.parse::<Expr>()?),
                "time_type" => config.time_type = Some(input.parse::<Type>()?),
                "bytes" => config.bytes = Some(input.parse::<Expr>()?),
                other => {
                    return Err(Error::new(key.span(), format!("unknown contime_event option `{other}`")));
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(config)
    }
}

impl Parse for SnapshotDeriveConfig {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut config = Self { events: None, ids: None, time: None, time_type: None, bytes: None, compact: None, apply: None };
        while !input.is_empty() {
            let key = input.parse::<Ident>()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "events" => {
                    let content;
                    syn::bracketed!(content in input);
                    config.events = Some(Punctuated::<Path, Token![,]>::parse_terminated(&content)?.into_iter().collect());
                }
                "id" => {
                    let content;
                    syn::bracketed!(content in input);
                    config.ids = Some(Punctuated::<Ident, Token![,]>::parse_terminated(&content)?.into_iter().collect());
                }
                "time" => config.time = Some(input.parse::<Expr>()?),
                "time_type" => config.time_type = Some(input.parse::<Type>()?),
                "bytes" => config.bytes = Some(input.parse::<Expr>()?),
                "compact" => config.compact = Some(input.parse::<Block>()?),
                "apply" => config.apply = Some(input.parse::<Block>()?),
                other => {
                    return Err(Error::new(key.span(), format!("unknown contime_snapshot option `{other}`")));
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(config)
    }
}
