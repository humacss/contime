//! Deduplicate concrete lanes and generate aggregate enums and trait adapters.
use super::metadata::State;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::HashSet;
use syn::Path;
pub(super) fn generate(mut state: State) -> syn::Result<TokenStream> {
    let root = crate::expansion::root();
    let vis = &state.vis;
    let name = &state.name;
    let time = &state.time;
    let mut identity_checks = quote!();
    // Registration keys select candidates; Rust proves nominal type equality before deduplication.
    for lanes in [&mut state.events, &mut state.snapshots] {
        let mut identities = std::collections::HashMap::<String, Path>::new();
        lanes.retain(|lane| {
            if let Some(previous) = identities.get(&lane.key.value()) {
                let candidate = &lane.path;
                identity_checks.extend(quote! {
                    const _: fn() = || {
                        fn __contime_same_lane<T>(_: *mut T, _: *mut T) {}
                        __contime_same_lane(::core::ptr::null_mut::<#previous>(), ::core::ptr::null_mut::<#candidate>());
                    };
                });
                false
            } else {
                identities.insert(lane.key.value(), lane.path.clone());
                true
            }
        });
        let mut names = HashSet::new();
        for lane in lanes {
            if !names.insert(lane.name.to_string()) {
                return Err(syn::Error::new_spanned(
                    &lane.path,
                    "distinct ConTime lanes use the same variant name; import one type under a different alias",
                ));
            }
        }
    }
    let en = format_ident!("{}EventLanes", name);
    let sn = format_ident!("{}SnapshotLanes", name);
    let ev = state.events.iter().map(|l| &l.name).collect::<Vec<_>>();
    let et = state.events.iter().map(|l| &l.path).collect::<Vec<_>>();
    let sv = state.snapshots.iter().map(|l| &l.name).collect::<Vec<_>>();
    let st = state.snapshots.iter().map(|l| &l.path).collect::<Vec<_>>();
    let mut views = Vec::new();
    for (index, ty) in et.iter().enumerate() {
        let variant = ev[index];
        views.push(quote!(impl #root::module::View<#ty> for #en {fn get(&self)->Option<&#ty>{match self {Self::#variant(event)=>Some(event),_=>None}}}));
    }
    let event_match = if ev.is_empty() { quote!(*self) } else { quote!(self) };
    let snapshot_match = if sv.is_empty() { quote!(*self) } else { quote!(self) };
    Ok(quote! {
        #identity_checks
        #[doc = "Owned event lanes accepted by this ConTime module."]
        #vis enum #en {#(#[doc = "A registered event payload."] #ev(#et),)*}
        #[doc = "Snapshot lanes returned by this ConTime module."]
        #[derive(Clone)] #vis enum #sn {#(#[doc = "A registered snapshot state."] #sv(#st),)*}
        #(impl From<#et> for #en {fn from(event:#et)->Self {Self::#ev(event)}})*
        #(impl From<#st> for #sn {fn from(snapshot:#st)->Self {Self::#sv(snapshot)}})*
        #(#views)*
        impl #root::checkpoints::Event for #en {type Time=#time;fn time(&self)->Self::Time{match #event_match {#(Self::#ev(event)=>#root::checkpoints::Event::time(event),)*}}}
        impl #root::Input for #en {fn event_id(&self)->u128 {match #event_match{#(Self::#ev(event)=>#root::module::Event::event_id(event),)*}}fn snapshot_ids(&self,emit:&mut impl FnMut(u128)){#(<#st as #root::module::Routes<Self>>::snapshot_ids(self,emit);)*}}
        impl #root::checkpoints::Snapshot for #sn {type Time=#time;fn time(&self)->&Self::Time{match #snapshot_match {#(Self::#sv(snapshot)=>#root::checkpoints::Snapshot::time(snapshot),)*}}fn set_time(&mut self,time:Self::Time){match #snapshot_match {#(Self::#sv(snapshot)=>#root::checkpoints::Snapshot::set_time(snapshot,time),)*}}}
        impl #root::checkpoints::ApplyEvents<#en> for #sn {
            fn create(snapshot_id:u128,event:&#en)->Self {#(if let Some(snapshot)=<#st as #root::module::Routes<#en>>::create(snapshot_id,event){return Self::#sv(snapshot);})* panic!("event does not route to requested snapshot")}
            fn apply_events(&mut self,batch:#root::checkpoints::ApplyBatch<'_, '_,Self::Time,#en>){match #snapshot_match{#(Self::#sv(snapshot)=><#st as #root::module::Apply<#en>>::apply(snapshot,batch),)*}}
        }
        impl #root::module::Module for #name {type Time=#time;type EventLanes=#en;type SnapshotLanes=#sn;}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn duplicate_state() -> State {
        syn::parse_quote! { App; i64; ["event" Added=first::Added,"event" Added=second::Added] [] [] [] }
    }
    #[test]
    fn duplicate_registration_candidates_require_rust_type_equality() {
        let generated = generate(duplicate_state()).unwrap();
        let parsed: syn::File = syn::parse2(generated).unwrap();
        let assertion = parsed
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Const(item) => Some(item),
                _ => None,
            })
            .unwrap();
        let expression = &assertion.expr;
        let expression = quote!(#expression).to_string();
        assert!(expression.contains("null_mut :: < first :: Added >"));
        assert!(expression.contains("null_mut :: < second :: Added >"));
        assert!(expression.contains("* mut T"));
    }
    #[test]
    fn distinct_registration_names_are_diagnosed() {
        let collision = syn::parse_quote!(App;i64;["one" Added=first::Added,"two" Added=second::Added] [] [] []);
        assert!(generate(collision).unwrap_err().to_string().contains("same variant name"));
    }
    #[test]
    #[ignore = "Criterion composition benchmark; run explicitly with --ignored --nocapture"]
    fn benchmark_lane_composition() {
        let mut criterion = criterion::Criterion::default();
        criterion.bench_function("macros/lane_composition", |bench| {
            bench.iter_batched(duplicate_state, |state| criterion::black_box(generate(state).unwrap()), criterion::BatchSize::SmallInput)
        });
        criterion.final_summary();
    }
}
