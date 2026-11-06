//! Deduplicate concrete lanes and generate aggregate enums and trait adapters.
use super::metadata::State;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
pub(super) fn generate(mut state: State) -> syn::Result<TokenStream> {
    let root = crate::expansion::root();
    let vis = &state.vis;
    let name = &state.name;
    let time = &state.time;
    let mut identity_checks = quote!();
    // Registration names select candidates; Rust proves type equality before deduplication.
    for lanes in [&mut state.events, &mut state.snapshots] {
        let mut unique: Vec<super::metadata::Lane> = Vec::new();
        for lane in lanes.drain(..) {
            if let Some(previous) = unique.iter_mut().find(|entry| entry.name == lane.name) {
                let a = &previous.path;
                let b = &lane.path;
                identity_checks.extend(quote!(const _:fn()=||{fn same<T>(_:*mut T,_:*mut T){} same(::core::ptr::null_mut::<#a>(),::core::ptr::null_mut::<#b>());};));
                match (&mut previous.subset, lane.subset) {
                    (Some(existing), Some(additional)) => {
                        for event in additional {
                            if let Some(old) = existing.iter().find(|old| old.name == event.name) {
                                let a = &old.path;
                                let b = &event.path;
                                identity_checks.extend(quote!(const _:fn()=||{fn same<T>(_:*mut T,_:*mut T){} same(::core::ptr::null_mut::<#a>(),::core::ptr::null_mut::<#b>());};));
                            } else {
                                existing.push(event);
                            }
                        }
                    }
                    _ => previous.subset = None,
                }
            } else {
                unique.push(lane);
            }
        }
        *lanes = unique;
    }
    let en = format_ident!("{}EventLanes", name);
    let sn = format_ident!("{}SnapshotLanes", name);
    let ev = state.events.iter().map(|l| &l.name).collect::<Vec<_>>();
    let et = state.events.iter().map(|l| &l.path).collect::<Vec<_>>();
    let sv = state.snapshots.iter().map(|l| &l.name).collect::<Vec<_>>();
    let st = state.snapshots.iter().map(|l| &l.path).collect::<Vec<_>>();
    let set_initializers = state
        .snapshots
        .iter()
        .map(|lane| {
            let snapshot = &lane.path;
            let base = quote!(#root::module::EventSet::<#snapshot>::new());
            if let Some(subset) = &lane.subset {
                let events = subset.iter().map(|event| &event.path);
                quote!(#base.with_event_types(&[#(::std::any::TypeId::of::<#events>()),*]))
            } else {
                base
            }
        })
        .collect::<Vec<_>>();
    let sets = (0..st.len()).map(|index| format_ident!("__contime_events_{}", index)).collect::<Vec<_>>();
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
        impl #en {
            #(fn #sets() -> &'static #root::module::EventSet<#st> {
                static SET: ::std::sync::OnceLock<#root::module::EventSet<#st>> = ::std::sync::OnceLock::new();
                SET.get_or_init(|| {
                    let set = #set_initializers;
                    set.validate::<Self>();
                    set
                })
            })*
        }
        impl #root::module::EventSource for #en {
            fn event_ref(&self)->#root::module::EventRef<'_> {match #event_match {#(Self::#ev(event)=>#root::module::EventRef::new(event),)*}}
            fn get<__E: ::std::any::Any>(&self)->Option<&__E> {
                match #event_match {#(Self::#ev(event)=>(event as &dyn ::std::any::Any).downcast_ref::<__E>(),)*}
            }
            fn contains_event_type(_id: ::std::any::TypeId)->bool {false #( || _id == ::std::any::TypeId::of::<#et>())*}
        }
        impl #root::checkpoints::Event for #en {type Time=#time;fn time(&self)->Self::Time{match #event_match {#(Self::#ev(event)=>#root::checkpoints::Event::time(event),)*}}}
        impl #root::Input for #en {
            const TRACKS_DUPLICATE_ADMISSIONS: bool = true;
            #[inline(always)] fn has_custom_duplicate_policies()->bool {
                use #root::module::EventPolicyTypeDispatch as _;
                false #( || (&#root::module::EventPolicyType::<#et>(::std::marker::PhantomData)).is_custom() )*
            }
            fn event_id(&self)->u128 {match #event_match{#(Self::#ev(event)=>#root::EventId::event_id(event),)*}}
            fn snapshot_ids(&self,_emit:&mut impl FnMut(u128)){#(Self::#sets().snapshot_ids(#root::module::EventSource::event_ref(self),_emit);)*}
            fn duplicate_policy_key(&self)->::std::any::TypeId { match #event_match {#(Self::#ev(_)=>::std::any::TypeId::of::<#et>(),)*} }
            #[inline(always)] fn track_duplicate_admissions(&self)->bool {use #root::module::EventPolicyDispatch as _; match #event_match {#(Self::#ev(event)=>(&#root::module::EventPolicyBridge(event)).policy_track(),)*}}
            fn create_duplicate_context(&self)->Box<dyn ::std::any::Any> {use #root::module::EventPolicyDispatch as _; match #event_match {#(Self::#ev(event)=>(&#root::module::EventPolicyBridge(event)).policy_context(),)*}}
            fn admit_duplicate_context(&self, context:&mut dyn ::std::any::Any) {use #root::module::EventPolicyDispatch as _;match #event_match {#(Self::#ev(event)=>(&#root::module::EventPolicyBridge(event)).policy_admit(context),)*}}
            fn prune_duplicate_context(&self, context:&mut dyn ::std::any::Any) {use #root::module::EventPolicyDispatch as _;match #event_match {#(Self::#ev(event)=>(&#root::module::EventPolicyBridge(event)).policy_prune(context),)*}}
            fn select_duplicate(&self,context:&mut dyn ::std::any::Any)->Option<#root::DuplicateDecision<Self>> {
                use #root::module::EventPolicyDispatch as _;
                match #event_match {#(Self::#ev(event)=>(&#root::module::EventPolicyBridge(event)).policy_select(context).map(|decision| match decision {
                    #root::DuplicateDecision::KeepExisting => #root::DuplicateDecision::KeepExisting,
                    #root::DuplicateDecision::ReplaceExisting { existing_time, event } => #root::DuplicateDecision::ReplaceExisting { existing_time, event: Self::#ev(event) },
                }),)*}
            }

        }
        impl #root::checkpoints::Snapshot for #sn {type Time=#time;fn time(&self)->&Self::Time{match #snapshot_match {#(Self::#sv(snapshot)=>#root::checkpoints::Snapshot::time(snapshot),)*}}fn set_time(&mut self,_time:Self::Time){match #snapshot_match {#(Self::#sv(snapshot)=>#root::checkpoints::Snapshot::set_time(snapshot,_time),)*}}}
        impl #root::checkpoints::ApplyEvents<#en> for #sn {
            fn create(_snapshot_id:u128,_event:&#en)->Self {#(if let Some(snapshot)=#en::#sets().create(_snapshot_id,#root::module::EventSource::event_ref(_event)){return Self::#sv(snapshot);})* panic!("event does not route to requested snapshot")}
            // Preserve iterator specialization through this generated delegation
            // and the shared application bridge, rather than adding a call boundary.
            #[inline(always)]
            fn apply_events(&mut self,_batch:#root::checkpoints::ApplyBatch<'_, '_,Self::Time,#en>){match #snapshot_match{#(Self::#sv(snapshot)=>#root::module::apply_batch(snapshot,_batch),)*}}
        }
        impl #root::module::Module for #name {fn validate(){#(#en::#sets();)*} type Time=#time;type EventLanes=#en;type SnapshotLanes=#sn;}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn duplicate_state() -> State {
        syn::parse_quote! { App; i64; [Added=first::Added,Added=second::Added] [] [] [] }
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
    fn different_registration_names_remain_distinct_flat_variants() {
        let state = syn::parse_quote!(App;i64;[First=first::Added,Second=second::Added] [] [] []);
        let parsed: syn::File = syn::parse2(generate(state).unwrap()).unwrap();
        let lanes = parsed
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Enum(item) if item.ident == "AppEventLanes" => Some(item),
                _ => None,
            })
            .unwrap();
        let names = lanes.variants.iter().map(|v| v.ident.to_string()).collect::<Vec<_>>();
        assert_eq!(names, ["First", "Second"]);
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
