//! Hidden proc-macro helpers for `contime`.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Error, Ident, Path, Result, Token, Type, parse_macro_input};

#[proc_macro]
pub fn __lanes_merge(input: TokenStream) -> TokenStream {
    match expand_lanes(parse_macro_input!(input as LanesManifest)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro]
pub fn fragment(input: TokenStream) -> TokenStream {
    match expand_fragment(parse_macro_input!(input as FragmentDecl)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_lanes(input: LanesManifest) -> Result<TokenStream2> {
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

                impl TryFrom<SnapshotLanes> for #ty {
                    type Error = SnapshotLanes;

                    fn try_from(lane: SnapshotLanes) -> Result<Self, Self::Error> {
                        match lane {
                            SnapshotLanes::#variant(snapshot) => Ok(snapshot),
                            other => Err(other),
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
                Self::#key(e) => <#event_ty as ::contime::ApplyEvent<#target_ty>>::snapshot_id(e),
            }
        })
        .collect::<Vec<_>>();

    let mut apply_pairs = Vec::new();
    for route in &routes {
        let key = &route.key;
        let event_ty = &route.event_ty;
        for target in &route.targets {
            let target_variant = &target.variant;
            let target_ty = &target.path;
            apply_pairs.push(quote! {
                (Self::#key(e), SnapshotLanes::#target_variant(s)) => {
                    <#event_ty as ::contime::ApplyEvent<#target_ty>>::apply_to(e, s)
                }
            });
        }
    }

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

            impl ::contime::ApplyEvent<SnapshotLanes> for EventLanes {
                fn snapshot_id(&self) -> u128 {
                    match self {
                        #( #event_snapshot_id_arms )*
                    }
                }

                fn apply_to(&self, snapshot: &mut SnapshotLanes) {
                    match (self, snapshot) {
                        #( #apply_pairs, )*
                        #[allow(unreachable_patterns)]
                        _ => {}
                    }
                }
            }

            impl ::contime::EventLanes<SnapshotLanes> for EventLanes {
                fn snapshots(&self) -> Vec<SnapshotLanes> {
                    match self {
                        #( #event_snapshots_arms )*
                    }
                }
            }

            #( #event_from_impls )*

            pub type Contime = ::contime::Contime<SnapshotLanes, EventLanes>;
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
    snapshots: Vec<Path>,
    routes: Vec<RouteEntry>,
}

struct FragmentDecl {
    name: Ident,
    snapshots: Vec<Path>,
    routes: Vec<RouteEntry>,
}

impl Parse for FragmentDecl {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse::<Token![macro]>()?;
        let name = input.parse::<Ident>()?;
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
            name,
            snapshots,
            routes,
        })
    }
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
            snapshots,
            routes,
        })
    }
}

struct RouteEntry {
    key: Ident,
    event_ty: Type,
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

fn expand_fragment(input: FragmentDecl) -> Result<TokenStream2> {
    let snapshots = input
        .snapshots
        .iter()
        .map(|snapshot| format!("{},", snapshot.to_token_stream()))
        .collect::<Vec<_>>()
        .join("\n");
    let routes = input
        .routes
        .iter()
        .map(|route| {
            let targets = route
                .targets
                .iter()
                .map(|target| target.to_token_stream().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{}({}) => [{}],",
                route.key,
                route.event_ty.to_token_stream(),
                targets
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let source = format!(
        r#"
            macro_rules! {name} {{
                (
                    @append
                    mod $modname:ident;
                    snapshots {{ $( $existing_snapshot:tt )* }}
                    routes {{ $( $existing_route:tt )* }}
                    fragments [ $( $rest:ident ),* $(,)? ]
                ) => {{
                    ::contime::__contime_collect_fragments! {{
                        @collect
                        mod $modname;
                        snapshots {{
                            $( $existing_snapshot )*
                            {snapshots}
                        }}
                        routes {{
                            $( $existing_route )*
                            {routes}
                        }}
                        fragments [ $( $rest ),* ]
                    }}
                }};
            }}

            pub(crate) use {name};
        "#,
        name = input.name,
        snapshots = snapshots,
        routes = routes,
    );
    TokenStream2::from_str(&source).map_err(|error| {
        Error::new(
            Span::call_site(),
            format!("failed to generate fragment macro: {error}"),
        )
    })
}
