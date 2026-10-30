//! Definition-site aliases and the cross-crate metadata callback protocol.
use crate::attributes::{variant, Options};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    DeriveInput, Ident, Path, Token, Type, Visibility,
};
fn sibling(path: &Path, prefix: &str) -> Path {
    let mut path = path.clone();
    let leaf = path.segments.last_mut().unwrap();
    leaf.ident = format_ident!("{}{}", prefix, leaf.ident);
    path
}
fn definition_path(path: &Path) -> Path {
    if path.leading_colon.is_some() || path.segments.first().is_some_and(|segment| segment.ident == "crate") {
        return path.clone();
    }
    let mut path = path.clone();
    if path.segments.first().is_some_and(|segment| segment.ident == "self") {
        path.segments.first_mut().unwrap().ident = syn::parse_quote!(super);
        return path;
    }
    if path.segments.len() == 1 || path.segments.first().is_some_and(|segment| segment.ident == "super") {
        path.segments.insert(0, syn::parse_quote!(super));
    }
    path
}
fn list<T: Parse>(input: ParseStream<'_>) -> syn::Result<Vec<T>> {
    let content;
    syn::bracketed!(content in input);
    Ok(content.parse_terminated(T::parse, Token![,])?.into_iter().collect())
}
pub(super) struct Lane {
    pub(super) key: syn::LitStr,
    pub(super) name: Ident,
    pub(super) path: Path,
}
impl Parse for Lane {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            key: input.parse()?,
            name: input.parse()?,
            path: {
                input.parse::<Token![=]>()?;
                input.parse()?
            },
        })
    }
}
pub(super) struct Pending {
    pub(super) callback: Path,
    pub(super) prefix: Path,
    pub(super) ancestors: Vec<Ident>,
}
impl Parse for Pending {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let content;
        syn::parenthesized!(content in input);
        Ok(Self {
            callback: content.parse()?,
            prefix: {
                content.parse::<Token![,]>()?;
                content.parse()?
            },
            ancestors: {
                content.parse::<Token![,]>()?;
                list(&content)?
            },
        })
    }
}
pub struct State {
    pub(super) vis: Visibility,
    pub(super) name: Ident,
    pub(super) time: Type,
    pub(super) events: Vec<Lane>,
    pub(super) snapshots: Vec<Lane>,
    pub(super) pending: Vec<Pending>,
    pub(super) seen: Vec<Ident>,
    pub(super) current: Option<Current>,
}
pub(super) struct Current {
    pub(super) key: Ident,
    pub(super) ancestors: Vec<Ident>,
    pub(super) events: Vec<Lane>,
    pub(super) snapshots: Vec<Lane>,
    pub(super) pending: Vec<Pending>,
    pub(super) time: Type,
}
impl Parse for State {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let vis = input.parse()?;
        let name = input.parse()?;
        input.parse::<Token![;]>()?;
        let time = input.parse()?;
        input.parse::<Token![;]>()?;
        let events = list(input)?;
        let snapshots = list(input)?;
        let pending = list(input)?;
        let seen = list(input)?;
        let current = if input.is_empty() {
            None
        } else {
            Some(Current {
                key: input.parse()?,
                ancestors: list(input)?,
                time: input.parse()?,
                events: list(input)?,
                snapshots: list(input)?,
                pending: list(input)?,
            })
        };
        Ok(Self { vis, name, time, events, snapshots, pending, seen, current })
    }
}
pub(super) fn lane_tokens(lanes: &[Lane]) -> Vec<TokenStream> {
    lanes
        .iter()
        .map(|l| {
            let k = &l.key;
            let n = &l.name;
            let p = &l.path;
            quote!(#k #n=#p)
        })
        .collect()
}
pub(super) fn pending_tokens(pending: &[Pending]) -> Vec<TokenStream> {
    pending
        .iter()
        .map(|p| {
            let c = &p.callback;
            let r = &p.prefix;
            let a = &p.ancestors;
            quote!((#c,#r,[#(#a),*]))
        })
        .collect()
}

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    register(Registration { input, keys: Vec::new(), key: None })
}
pub struct Registration {
    input: DeriveInput,
    keys: Vec<syn::LitStr>,
    pub(super) key: Option<syn::LitStr>,
}
impl Parse for Registration {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let key = Some(input.parse()?);
        let definition;
        syn::braced!(definition in input);
        Ok(Self { input: definition.parse()?, keys: list(input)?, key })
    }
}
pub fn register(mut state: Registration) -> syn::Result<TokenStream> {
    if let Some(key) = state.key.take() {
        state.keys.push(key);
    }
    let options = Options::read(&state.input, true)?;
    let paths = options.events.iter().map(|event| event.event.clone()).chain(options.snapshots.iter().cloned()).collect::<Vec<_>>();
    if let Some(path) = paths.get(state.keys.len()) {
        let mut metadata_path = path.clone();
        for segment in &mut metadata_path.segments {
            segment.arguments = syn::PathArguments::None;
        }
        let root = crate::expansion::root();
        let input = &state.input;
        let keys = &state.keys;
        return Ok(quote!(#metadata_path!(@contime_definition #root::__contime_register_module; {#input} [#(#keys),*]);));
    }
    derive_registered(state.input, state.keys)
}
fn derive_registered(input: DeriveInput, keys: Vec<syn::LitStr>) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(&input.generics, "module markers currently require concrete types"));
    }
    let options = Options::read(&input, true)?;
    let root = crate::expansion::root();
    let name = &input.ident;
    let vis = &input.vis;
    let time = options.time_type.ok_or_else(|| syn::Error::new_spanned(name, "module requires time = TimestampType"))?;
    let key = format_ident!("__contime_key_{}_{}", name, crate::registration::suffix(&crate::registration::key(name)));
    let exported = format_ident!("__contime_metadata_{}", key);
    let callback = format_ident!("__contime_module_{}", name);
    let types = format_ident!("__contime_types_{}", name);
    let mut aliases = Vec::new();
    let mut events = Vec::new();
    let mut snapshots = Vec::new();
    let mut deps = Vec::new();
    for (index, event) in options.events.iter().enumerate() {
        let alias = format_ident!("Event{}", index);
        let n = variant(&event.event);
        let path = definition_path(&event.event);
        aliases.push(quote!(pub type #alias=#path;));
        let key = concrete_key(&keys[index], &event.event);
        events.push(quote!(#key #n=$($prefix)*::#alias));
    }
    for (index, path) in options.snapshots.iter().enumerate() {
        let alias = format_ident!("Snapshot{}", index);
        let n = variant(path);
        let path = definition_path(path);
        aliases.push(quote!(pub type #alias=#path;));
        let key = concrete_key(&keys[options.events.len() + index], &path);
        snapshots.push(quote!(#key #n=$($prefix)*::#alias));
    }
    for (index, path) in options.modules.iter().enumerate() {
        let alias = format_ident!("Module{}", index);
        let type_alias = format_ident!("Dependency{}", index);
        let dep_callback = definition_path(&sibling(path, "__contime_module_"));
        let dep_types = definition_path(&sibling(path, "__contime_types_"));
        aliases.push(quote!(pub use #dep_callback as #alias;pub use #dep_types as #type_alias;));
        deps.push(quote!(($($prefix)*::#alias,$($prefix)*::#type_alias,[$($ancestors,)* #key])));
    }
    // Public aliases retain definition-site resolution when metadata is expanded elsewhere.
    Ok(quote! {
        #[doc(hidden)] #[allow(non_snake_case,non_camel_case_types)] pub mod #types {use super::*;pub type __ContimeTime=#time;#(#aliases)*}
        #[doc(hidden)] #[macro_export] macro_rules! #exported {
            ($collector:path,[$($prefix:tt)*],[$($ancestors:ident),*],[$($state:tt)*])=>{
                $collector!{$($state)* #key [$($ancestors),*] $($prefix)*::__ContimeTime [#(#events),*] [#(#snapshots),*] [#(#deps),*]}
            }
        }
        #[doc(hidden)] pub use #exported as #callback;
         #callback!(#root::__contime_collect_module,[#types],[],[#vis #name;#time; [] [] [] []]);
    })
}
fn concrete_key(key: &syn::LitStr, path: &Path) -> syn::LitStr {
    let arguments = &path.segments.last().unwrap().arguments;
    syn::LitStr::new(&format!("{}{}", key.value(), quote!(#arguments)), key.span())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_time_alias_does_not_shadow_a_custom_time_type() {
        let input = syn::parse_quote! {#[contime_module(time=Time)] struct App;};
        let generated = derive(input).unwrap().to_string();
        assert!(generated.contains("pub type __ContimeTime"));
    }
}
