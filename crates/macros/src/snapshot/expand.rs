//! Assemble the validated snapshot definition, routes and application adapter.
use super::{borrowed_events, definition, routing};
use crate::attributes::Options;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::DeriveInput;
pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let options = Options::read(&input, false)?;
    let root = crate::expansion::root();
    let name = &input.ident;
    let vis = &input.vis;
    let definition::Definition { time, time_ty, id_fields, id_types, auto, generics: snapshot_generics } =
        definition::read(&input, &options)?;
    let (ig, tg, wc) = snapshot_generics.split_for_impl();
    let event_name = format_ident!("{}Event", name);
    let routes = &options.events;
    let apply = options.apply.as_ref().ok_or_else(|| syn::Error::new_spanned(name, "snapshot requires apply = { ... }"))?;
    let apply_statements = &apply.stmts;
    let implementations = routing::expand(&input, routes, &id_fields, &snapshot_generics)?;
    let borrowed_events::BorrowedEvents { declaration: borrowed_events, view_arms, event_bounds } =
        borrowed_events::expand(&input, routes, &event_name)?;
    let mut apply_generics = snapshot_generics.clone();
    apply_generics.params.push(syn::parse_quote!(__I));
    for bound in &event_bounds {
        apply_generics.make_where_clause().predicates.push(syn::parse_quote!(__I: #bound));
    }
    let (aig, _, awc) = apply_generics.split_for_impl();
    let routing=routes.iter().map(|route| {let event=&route.event;quote!{if let Some(event)=<__I as #root::module::View<#event>>::get(event) {<Self as #root::module::Route<#event>>::snapshot_ids(event,emit);}}});
    let creating=routes.iter().map(|route| {let event=&route.event;quote!{if let Some(event)=<__I as #root::module::View<#event>>::get(event) {if let Some(snapshot)=<Self as #root::module::Route<#event>>::create(snapshot_id,event) {return Some(snapshot);}}}});
    // The inferred input lane is generic, while each application item only borrows its payload.
    let metadata = crate::registration::metadata(name);
    Ok(quote! {
        #metadata
        #auto
        impl #ig #name #tg #wc { #[doc = "Constructs this snapshot type's namespaced logical identity."] #vis fn snapshot_id(#(#id_fields:#id_types),*)->u128 { #root::module::identity::<Self,_>(&(#(#id_fields,)*)) } }
        impl #ig #root::checkpoints::Snapshot for #name #tg #wc {type Time=#time_ty;fn time(&self)->&Self::Time {&#time} fn set_time(&mut self,time:Self::Time){#time=time;}}
        #borrowed_events
        #implementations
        impl #aig #root::module::Routes<__I> for #name #tg #awc {
            fn snapshot_ids(event:&__I,emit:&mut impl FnMut(u128)) {#(#routing)*}
            fn create(snapshot_id:u128,event:&__I)->Option<Self> {#(#creating)* None}
        }
        impl #aig #root::module::Apply<__I> for #name #tg #awc {
            fn apply(&mut self,__batch:#root::checkpoints::ApplyBatch<'_, '_, Self::Time,__I>) {
                let __events=__batch.events.filter_map(|event| {#view_arms None});
                let batch=#root::module::ApplyBatch {snapshot_id:__batch.snapshot_id,time:__batch.time,history_event_count:__batch.history_event_count,events:__events};
                #(#apply_statements)*
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn generic_snapshot() -> DeriveInput {
        syn::parse_quote! {
            #[contime_snapshot(id=[self.owner],events=[Added<T>],apply={for _ in batch.events {}})]
            struct Counter<T:Clone+Default> {time:i64,owner:u64,value:T}
        }
    }
    #[test]
    #[ignore = "Criterion expansion benchmark; run explicitly with --ignored --nocapture"]
    fn benchmark_snapshot_expansion() {
        let input = generic_snapshot();
        let mut criterion = criterion::Criterion::default();
        criterion.bench_function("macros/snapshot_expansion", |bench| {
            bench.iter_batched(|| input.clone(), |input| criterion::black_box(expand(input).unwrap()), criterion::BatchSize::SmallInput)
        });
        criterion.final_summary();
    }
}
