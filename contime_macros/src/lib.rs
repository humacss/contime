//! Hidden proc-macro helpers for `contime`.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Block, DeriveInput, Error, Expr, Fields, Ident, Path, Result, Token, Type, parse_macro_input,
};

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
    validate_route_targets(&snapshots, &routes)?;

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

    let snapshot_from_event_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            let target = route
                .targets
                .first()
                .expect("merged route should always have at least one target");
            let target_variant = &target.variant;
            let target_ty = &target.path;
            quote! {
                EventLanes::#key(e) => {
                    SnapshotLanes::#target_variant(
                        <#target_ty as ::contime::SeedSnapshot<#event_ty>>::seed_from_event(e)
                    )
                }
            }
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

    let event_id_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            quote! { Self::#key(e) => <#event_ty as ::contime::Event>::id(e), }
        })
        .collect::<Vec<_>>();

    let event_time_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            quote! { Self::#key(e) => <#event_ty as ::contime::Event>::time(e), }
        })
        .collect::<Vec<_>>();

    let event_size_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            quote! { Self::#key(e) => <#event_ty as ::contime::Event>::conservative_size(e), }
        })
        .collect::<Vec<_>>();

    let event_snapshot_id_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            let target = route
                .targets
                .first()
                .expect("merged route should always have at least one target");
            let target_ty = &target.path;
            quote! {
                Self::#key(e) => <#event_ty as ::contime::SnapshotEvent<#target_ty>>::snapshot_id(e),
            }
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
                #target_ty: ::contime::ApplyEvents
            });
            apply_bounds.push(quote! {
                <#target_ty as ::contime::Snapshot>::Event: From<#event_ty>
            });
        }
    }

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
                        for event in batch.events.iter().copied() {
                            if let EventLanes::#key(event) = event {
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
                        <#snapshot_ty as ::contime::ApplyEvents>::apply_events(
                            snapshot,
                            ::contime::ApplyBatch {
                                snapshot_id: batch.snapshot_id,
                                time: batch.time,
                                events: &bucket,
                            },
                        );
                    } else {
                        <Self as ::contime::Snapshot>::set_time(self, batch.time);
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    let event_snapshots_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            let targets = route.targets.iter().map(|target| {
                let target_variant = &target.variant;
                let target_ty = &target.path;
                quote! {
                    {
                        SnapshotLanes::#target_variant(
                            <#target_ty as ::contime::SeedSnapshot<#event_ty>>::seed_from_event(e)
                        )
                    }
                }
            });
            quote! {
                Self::#key(e) => {
                    vec![
                        #( #targets, )*
                    ]
                }
            }
        })
        .collect::<Vec<_>>();

    let event_routed_snapshots_arms = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            let targets = route.targets.iter().map(|target| {
                let target_variant = &target.variant;
                let target_ty = &target.path;
                quote! {
                    {
                        ::contime::RoutedSnapshot {
                            snapshot_id: <#event_ty as ::contime::SnapshotEvent<#target_ty>>::snapshot_id(e),
                            initial_snapshot: SnapshotLanes::#target_variant(
                                <#target_ty as ::contime::SeedSnapshot<#event_ty>>::seed_from_event(e)
                            ),
                        }
                    }
                }
            });
            quote! {
                Self::#key(e) => {
                    vec![
                        #( #targets, )*
                    ]
                }
            }
        })
        .collect::<Vec<_>>();

    let event_from_impls = routes
        .iter()
        .map(|route| {
            let key = &route.key;
            let event_ty = &route.event_ty;
            quote! {
                impl From<#event_ty> for EventLanes {
                    fn from(event: #event_ty) -> Self {
                        Self::#key(event)
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    let modname = input.modname;
    let context_ty = input
        .context
        .map(|context| quote! { #context })
        .unwrap_or_else(|| quote! { () });

    Ok(quote! {
        mod #modname {
            use super::*;

            #[derive(Clone, Debug, PartialEq, Eq)]
            pub enum SnapshotLanes {
                #( #snapshot_variants )*
            }

            impl ::contime::SnapshotLanes for SnapshotLanes {}

            impl ::contime::Snapshot for SnapshotLanes {
                type Event = EventLanes;

                fn id(&self) -> u128 {
                    match self {
                        #( #snapshot_id_arms )*
                    }
                }

                fn time(&self) -> i64 {
                    match self {
                        #( #snapshot_time_arms )*
                    }
                }

                fn set_time(&mut self, time: i64) {
                    match self {
                        #( #snapshot_set_time_arms )*
                    }
                }

                fn conservative_size(&self) -> u64 {
                    match self {
                        #( #snapshot_size_arms )*
                    }
                }

                fn from_event(event: &Self::Event) -> Self {
                    match event {
                        #( #snapshot_from_event_arms )*
                    }
                }
            }

            #( #snapshot_from_impls )*

            #[derive(Debug, Clone, Eq, PartialEq)]
            pub enum EventLanes {
                #( #event_variants )*
            }

            impl ::contime::Event for EventLanes {
                fn id(&self) -> u128 {
                    match self {
                        #( #event_id_arms )*
                    }
                }

                fn time(&self) -> i64 {
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

            impl ::contime::SnapshotEvent<SnapshotLanes> for EventLanes
            where
                #( #apply_bounds, )*
            {
                fn snapshot_id(&self) -> u128 {
                    match self {
                        #( #event_snapshot_id_arms )*
                    }
                }

            }

            impl ::contime::ApplyEvents for SnapshotLanes
            where
                #( #apply_bounds, )*
            {
                fn apply_events(&mut self, batch: ::contime::ApplyBatch<'_, Self::Event>) {
                    match self {
                        #( #apply_snapshot_arms )*
                    }
                }
            }

            impl<C> ::contime::EventLanes<SnapshotLanes, C> for EventLanes
            where
                EventLanes: ::contime::SnapshotEvent<SnapshotLanes>,
                SnapshotLanes: ::contime::ApplyEvents,
            {
                fn snapshots(&self) -> Vec<SnapshotLanes> {
                    match self {
                        #( #event_snapshots_arms )*
                    }
                }

                fn routed_snapshots(&self) -> Vec<::contime::RoutedSnapshot<SnapshotLanes>> {
                    match self {
                        #( #event_routed_snapshots_arms )*
                    }
                }
            }

            #( #event_from_impls )*

            pub type Contime = ::contime::Contime<SnapshotLanes, EventLanes, #context_ty>;
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
    let id = config
        .id
        .ok_or_else(|| Error::new(attr.span(), "`contime_event` requires `id = ...`"))?;
    let time = config
        .time
        .ok_or_else(|| Error::new(attr.span(), "`contime_event` requires `time = ...`"))?;
    let bytes = config
        .bytes
        .ok_or_else(|| Error::new(attr.span(), "`contime_event` requires `bytes = ...`"))?;

    Ok(quote! {
        impl ::contime::Event for #name {
            fn id(&self) -> u128 {
                #id
            }

            fn time(&self) -> i64 {
                #time
            }

            fn conservative_size(&self) -> u64 {
                #bytes
            }
        }
    })
}

fn expand_contime_snapshot(input: DeriveInput) -> Result<TokenStream2> {
    let name = input.ident;
    let attr = input
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("contime_snapshot"))
        .ok_or_else(|| Error::new(name.span(), "missing `#[contime_snapshot(...)]` attribute"))?;
    match input.data {
        syn::Data::Struct(data) => match data.fields {
            Fields::Named(_) => {}
            other => {
                return Err(Error::new(
                    other.span(),
                    "`ContimeSnapshot` currently requires a struct with named fields",
                ));
            }
        },
        other => {
            let _ = other;
            return Err(Error::new(
                name.span(),
                "`ContimeSnapshot` can only be derived for structs",
            ));
        }
    }

    let config = attr.parse_args::<SnapshotDeriveConfig>()?;
    let events = config
        .events
        .ok_or_else(|| Error::new(attr.span(), "`contime_snapshot` requires `events = [...]`"))?;
    if events.is_empty() {
        return Err(Error::new(attr.span(), "`contime_snapshot` requires at least one event"));
    }
    let ids = config
        .ids
        .ok_or_else(|| Error::new(attr.span(), "`contime_snapshot` requires `id = [...]`"))?;
    if ids.len() != 1 {
        return Err(Error::new(
            attr.span(),
            "`ContimeSnapshot` currently supports exactly one id field",
        ));
    }
    let id = ids
        .first()
        .expect("checked len")
        .clone();
    let time = config
        .time
        .ok_or_else(|| Error::new(attr.span(), "`contime_snapshot` requires `time = ...`"))?;
    let bytes = config
        .bytes
        .ok_or_else(|| Error::new(attr.span(), "`contime_snapshot` requires `bytes = ...`"))?;
    let apply = config
        .apply
        .ok_or_else(|| Error::new(attr.span(), "`contime_snapshot` requires `apply = { ... }`"))?;

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
            Ok(quote! { Self::#variant(event) => <#event as ::contime::Event>::id(event), })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_time_arms = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            Ok(quote! { Self::#variant(event) => <#event as ::contime::Event>::time(event), })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_size_arms = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            Ok(quote! { Self::#variant(event) => <#event as ::contime::Event>::conservative_size(event), })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_from_impls = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            Ok(quote! {
                impl From<#event> for #event_enum {
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
                impl ::contime::SnapshotEvent<#name> for #event {
                    fn snapshot_id(&self) -> u128 {
                        self.#id
                    }
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let seed_snapshot_impls = events
        .iter()
        .map(|event| {
            let id = &id;
            Ok(quote! {
                impl ::contime::SeedSnapshot<#event> for #name {
                    fn seed_from_event(event: &#event) -> Self {
                        Self {
                            #id: event.#id,
                            time: ::contime::Event::time(event),
                            ..Default::default()
                        }
                    }
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let from_event_arms = events
        .iter()
        .map(|event| {
            let variant = trailing_ident(event)?;
            let id = &id;
            Ok(quote! {
                #event_enum::#variant(event) => Self {
                    #id: event.#id,
                    time: ::contime::Event::time(event),
                    ..Default::default()
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum #event_enum {
            #( #event_variants )*
        }

        impl ::contime::Event for #event_enum {
            fn id(&self) -> u128 {
                match self {
                    #( #event_id_arms )*
                }
            }

            fn time(&self) -> i64 {
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

        #( #event_from_impls )*
        #( #event_snapshot_impls )*
        #( #seed_snapshot_impls )*

        impl ::contime::Snapshot for #name {
            type Event = #event_enum;

            fn id(&self) -> u128 {
                self.#id
            }

            fn time(&self) -> i64 {
                #time
            }

            fn set_time(&mut self, time: i64) {
                self.time = time;
            }

            fn conservative_size(&self) -> u64 {
                #bytes
            }

            fn from_event(event: &Self::Event) -> Self {
                match event {
                    #( #from_event_arms )*
                }
            }
        }

        impl ::contime::ApplyEvents for #name {
            fn apply_events(&mut self, batch: ::contime::ApplyBatch<'_, Self::Event>) {
                let batch = batch;
                #apply
            }
        }

        macro_rules! #snapshot_lanes_macro {
            (
                @ao_collect_enum
                enum $name:ident
                vis { $vis:vis }
                attrs { $($attrs:tt)* }
                variants { $($variants:tt)* }
                rest [ $next:path $(, $rest:path)* $(,)? ]
            ) => {
                $next! {
                    @ao_collect_enum
                    enum $name
                    vis { $vis }
                    attrs { $($attrs)* }
                    variants {
                        $($variants)*
                        #name(#name),
                    }
                    rest [ $($rest),* ]
                }
            };
        }

        macro_rules! #event_lanes_macro {
            (
                @ao_collect_enum
                enum $name:ident
                vis { $vis:vis }
                attrs { $($attrs:tt)* }
                variants { $($variants:tt)* }
                rest [ $next:path $(, $rest:path)* $(,)? ]
            ) => {
                $next! {
                    @ao_collect_enum
                    enum $name
                    vis { $vis }
                    attrs { $($attrs)* }
                    variants {
                        $($variants)*
                        #name(#event_enum),
                    }
                    rest [ $($rest),* ]
                }
            };
        }

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
                        #name,
                    }
                    event_routes {
                        $($event_routes)*
                        #event_enum(#event_enum) => #event_enum => [#name],
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
                    format!(
                        "snapshot variant `{variant_key}` would refer to multiple snapshot types: `{existing}` and `{type_key}`"
                    ),
                ));
            }
        }
        by_variant.insert(variant_key, type_key.clone());
        by_type.insert(
            type_key,
            SnapshotSpec {
                path: path.clone(),
                variant,
            },
        );
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
                    format!(
                        "event type `{event_key}` is routed under multiple keys: `{existing_key}` and `{key_name}`"
                    ),
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
                format!(
                    "route key `{key_name}` uses conflicting event types: `{}` and `{event_key}`",
                    entry.event_key
                ),
            ));
        }

        let mut seen = entry
            .targets
            .iter()
            .map(|target| normalized_path_key(&target.path))
            .collect::<BTreeSet<_>>();

        for target in &route.targets {
            let target_key = normalized_path_key(target);
            if seen.insert(target_key) {
                entry.targets.push(SnapshotSpec {
                    path: target.clone(),
                    variant: trailing_ident(target)?,
                });
            }
        }
    }

    if merged.is_empty() {
        return Err(Error::new(
            Span::call_site(),
            "contime::lanes! requires at least one route across the listed fragments",
        ));
    }

    Ok(merged.into_values().collect())
}

fn validate_route_targets(snapshots: &[SnapshotSpec], routes: &[RouteSpec]) -> Result<()> {
    let known = snapshots
        .iter()
        .map(|snapshot| normalized_path_key(&snapshot.path))
        .collect::<BTreeSet<_>>();

    for route in routes {
        for target in &route.targets {
            let target_key = normalized_path_key(&target.path);
            if !known.contains(&target_key) {
                return Err(Error::new(
                    target.path.span(),
                    format!(
                        "route target `{target_key}` is not listed in the assembled snapshots"
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn trailing_ident(path: &Path) -> Result<Ident> {
    path.segments
        .last()
        .map(|segment| segment.ident.clone())
        .ok_or_else(|| Error::new(path.span(), "expected a named path"))
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
    snapshots: Vec<Path>,
    routes: Vec<RouteEntry>,
}

struct NewLanesManifest {
    modname: Ident,
    context: Option<Type>,
    snapshots: Vec<Path>,
    routes: Vec<RouteEntry>,
}

struct EventDeriveConfig {
    id: Option<Expr>,
    time: Option<Expr>,
    bytes: Option<Expr>,
}

struct SnapshotDeriveConfig {
    events: Option<Vec<Path>>,
    ids: Option<Vec<Ident>>,
    time: Option<Expr>,
    bytes: Option<Expr>,
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
        let snapshots = Punctuated::<Path, Token![,]>::parse_terminated(&snapshots_content)?
            .into_iter()
            .collect::<Vec<_>>();

        let routes_label = input.parse::<Ident>()?;
        if routes_label != "routes" {
            return Err(Error::new(routes_label.span(), "expected `routes`"));
        }
        let routes_content;
        syn::braced!(routes_content in input);
        let routes = Punctuated::<RouteEntry, Token![,]>::parse_terminated(&routes_content)?
            .into_iter()
            .collect::<Vec<_>>();

        Ok(Self {
            modname,
            context: None,
            snapshots,
            routes,
        })
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

        let snapshots_label = input.parse::<Ident>()?;
        if snapshots_label != "snapshots" {
            return Err(Error::new(snapshots_label.span(), "expected `snapshots`"));
        }
        let snapshots_content;
        syn::bracketed!(snapshots_content in input);
        let snapshots = Punctuated::<Path, Token![,]>::parse_terminated(&snapshots_content)?
            .into_iter()
            .collect::<Vec<_>>();
        input.parse::<Token![;]>()?;

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

        Ok(Self {
            modname,
            context,
            snapshots,
            routes,
        })
    }
}

impl From<NewLanesManifest> for LanesManifest {
    fn from(value: NewLanesManifest) -> Self {
        Self {
            modname: value.modname,
            context: value.context,
            snapshots: value.snapshots,
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
        let targets = Punctuated::<Path, Token![,]>::parse_terminated(&targets_content)?
            .into_iter()
            .collect::<Vec<_>>();
        Ok(Self {
            key,
            event_ty,
            targets,
        })
    }
}

impl Parse for NewRouteEntry {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let event_ty = input.parse::<Path>()?;
        input.parse::<Token![=>]>()?;
        let targets_content;
        syn::bracketed!(targets_content in input);
        let targets = Punctuated::<Path, Token![,]>::parse_terminated(&targets_content)?
            .into_iter()
            .collect::<Vec<_>>();
        Ok(Self { event_ty, targets })
    }
}

impl From<NewRouteEntry> for RouteEntry {
    fn from(value: NewRouteEntry) -> Self {
        let key = trailing_ident(&value.event_ty).expect("parsed path has a trailing ident");
        let event_ty = Type::Path(syn::TypePath {
            qself: None,
            path: value.event_ty,
        });
        Self {
            key,
            event_ty,
            targets: value.targets,
        }
    }
}

impl Parse for EventDeriveConfig {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut config = Self {
            id: None,
            time: None,
            bytes: None,
        };
        while !input.is_empty() {
            let key = input.parse::<Ident>()?;
            input.parse::<Token![=]>()?;
            let expr = input.parse::<Expr>()?;
            match key.to_string().as_str() {
                "id" => config.id = Some(expr),
                "time" => config.time = Some(expr),
                "bytes" => config.bytes = Some(expr),
                other => {
                    return Err(Error::new(
                        key.span(),
                        format!("unknown contime_event option `{other}`"),
                    ));
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
        let mut config = Self {
            events: None,
            ids: None,
            time: None,
            bytes: None,
            apply: None,
        };
        while !input.is_empty() {
            let key = input.parse::<Ident>()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "events" => {
                    let content;
                    syn::bracketed!(content in input);
                    config.events = Some(
                        Punctuated::<Path, Token![,]>::parse_terminated(&content)?
                            .into_iter()
                            .collect(),
                    );
                }
                "id" => {
                    let content;
                    syn::bracketed!(content in input);
                    config.ids = Some(
                        Punctuated::<Ident, Token![,]>::parse_terminated(&content)?
                            .into_iter()
                            .collect(),
                    );
                }
                "time" => config.time = Some(input.parse::<Expr>()?),
                "bytes" => config.bytes = Some(input.parse::<Expr>()?),
                "apply" => config.apply = Some(input.parse::<Block>()?),
                other => {
                    return Err(Error::new(
                        key.span(),
                        format!("unknown contime_snapshot option `{other}`"),
                    ));
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(config)
    }
}
