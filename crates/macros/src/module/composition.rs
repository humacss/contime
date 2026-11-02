//! Traverse module dependencies and snapshot leaves, collecting flat metadata.
use super::lanes::generate;
use super::metadata::{lane_tokens, pending_tokens, State};
use proc_macro2::TokenStream;
use quote::quote;
pub fn collect(mut state: State) -> syn::Result<TokenStream> {
    let root = crate::expansion::root();
    let mut assertions = quote!();
    if let Some(mut current) = state.current.take() {
        if !current.snapshot && current.ancestors.iter().any(|k| k == &current.key) {
            return Err(syn::Error::new_spanned(current.key, "cycle in ConTime module dependencies"));
        }
        if current.snapshot || !state.seen.iter().any(|k| k == &current.key) {
            if !current.snapshot {
                state.seen.push(current.key);
            }
            let expected = &state.time;
            let actual = &current.time;
            assertions.extend(quote!(const _: ()={fn __contime_same_time(value:#actual)->#expected {value}};));
            if let Some(subset) = current.subset {
                let mut selected = Vec::new();
                let mut names = std::collections::HashSet::new();
                for requested in subset {
                    if !names.insert(requested.name.to_string()) {
                        return Err(syn::Error::new_spanned(requested.name, "duplicate event in snapshot subset"));
                    }
                    let Some(found) = current.events.iter().find(|event| event.name == requested.name) else {
                        return Err(syn::Error::new_spanned(requested.name, "event is not registered by this snapshot"));
                    };
                    let actual = &found.path;
                    let requested = &requested.path;
                    assertions.extend(quote!(const _:fn()=||{
                        fn same<T>(_:*mut T,_:*mut T){} same(::core::ptr::null_mut::<#actual>(),::core::ptr::null_mut::<#requested>());
                    };));
                    selected.push(found.clone());
                }
                for snapshot in &mut current.snapshots {
                    snapshot.subset = Some(selected.clone());
                }
                current.events = selected;
            }
            state.events.extend(current.events);
            state.snapshots.extend(current.snapshots);
            state.pending.extend(current.pending);
        }
    }
    if let Some(next) = state.pending.pop() {
        let callback = next.callback;
        let prefix = next.prefix;
        let ancestors = next.ancestors;
        let vis = &state.vis;
        let name = &state.name;
        let time = &state.time;
        let events = lane_tokens(&state.events);
        let snapshots = lane_tokens(&state.snapshots);
        let pending = pending_tokens(&state.pending);
        let seen = &state.seen;
        let extra = if let Some(snapshot) = next.snapshot {
            let n = snapshot.name;
            let path = snapshot.path;
            let selection = if let Some(subset) = snapshot.subset {
                let entries = lane_tokens(&subset);
                quote!(only [#(#entries),*])
            } else {
                quote!(all)
            };
            {
                quote!(,#n=#path,[#selection])
            }
        } else {
            quote!()
        };
        return Ok(quote! {
            #assertions #callback!(#root::__contime_collect_module,[#prefix],[#(#ancestors),*],
                [#vis #name;#time;[#(#events),*] [#(#snapshots),*] [#(#pending),*] [#(#seen),*]] #extra);
        });
    }
    let generated = generate(state)?;
    Ok(quote!(#assertions #generated))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependency_cycles_are_rejected() {
        let cycle = syn::parse_quote!(App;i64;[] [] [] [] module_key [module_key] i64 [] [] []);
        assert!(collect(cycle).unwrap_err().to_string().contains("cycle"));
    }
}
