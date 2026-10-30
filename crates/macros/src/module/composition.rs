//! Traverse module dependencies, rejecting cycles and collecting shared modules once.
use super::lanes::generate;
use super::metadata::{lane_tokens, pending_tokens, State};
use proc_macro2::TokenStream;
use quote::quote;
pub fn collect(mut state: State) -> syn::Result<TokenStream> {
    let root = crate::expansion::root();
    let mut assertions = quote!();
    if let Some(current) = state.current.take() {
        if current.ancestors.iter().any(|k| k == &current.key) {
            return Err(syn::Error::new_spanned(current.key, "cycle in ConTime module dependencies"));
        }
        if !state.seen.iter().any(|k| k == &current.key) {
            state.seen.push(current.key);
            let expected = &state.time;
            let actual = &current.time;
            assertions.extend(quote!(const _: ()={fn __contime_same_time(value:#actual)->#expected {value}};));
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
        return Ok(
            quote!(#assertions #callback!(#root::__contime_collect_module,[#prefix],[#(#ancestors),*],[#vis #name;#time;[#(#events),*] [#(#snapshots),*] [#(#pending),*] [#(#seen),*]]);),
        );
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
