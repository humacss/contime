//! Timestamp access, automatic hashing and event identity.
use crate::attributes::{bounds, fields, time, Options};
use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let options = Options::read(&input, false)?;
    if let Some(create) = &options.create {
        return Err(syn::Error::new_spanned(create, "create is a snapshot-only option"));
    }
    let id = crate::attributes::id_closure(&input, &options)?;
    if id.inputs.len() != 1 {
        return Err(syn::Error::new_spanned(id, "event id closure must accept one borrowed event"));
    }
    let root = crate::expansion::root();
    let name = &input.ident;
    let (time, time_ty) = time(&input, &options)?;
    let fields = fields(&input)?;
    let names = fields.iter().map(|(name, _)| name);
    let hash = if options.hash.unwrap_or(false) {
        let gen = bounds(input.generics.clone(), &fields, quote!(::core::hash::Hash));
        let (ig, tg, wc) = gen.split_for_impl();
        quote! { impl #ig ::core::hash::Hash for #name #tg #wc { fn hash<__H: ::core::hash::Hasher>(&self,state:&mut __H) { #(::core::hash::Hash::hash(&self.#names,state);)* } } }
    } else {
        quote!()
    };
    let mut event_generics = input.generics.clone();
    event_generics.make_where_clause().predicates.push(syn::parse_quote!(#time_ty: ::core::clone::Clone + ::core::cmp::PartialEq));
    let (ig, tg, wc) = event_generics.split_for_impl();
    let metadata = crate::registration::metadata(name);
    Ok(quote! {
        #metadata
        #hash
        impl #ig #root::checkpoints::Event for #name #tg #wc { type Time=#time_ty; fn time(&self)->Self::Time { #time.clone() } }
        impl #ig #name #tg #wc { #[doc = "Returns this event's identity for ordering and duplicate suppression."] pub fn event_id(&self)->u128 { <Self as #root::module::Event>::event_id(self) } }
        impl #ig #root::module::Event for #name #tg #wc { fn event_id(&self)->u128 { let __contime_id: fn(&Self)->u128 = #id; __contime_id(self) } }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_creation_is_not_an_event_option() {
        let input = syn::parse_quote! {
            #[contime_event(id=|event| event.id,create=|_id,event| event)]
            struct Added {time:i64,id:u128}
        };
        assert!(expand(input).unwrap_err().to_string().contains("snapshot-only"));
    }
    #[test]
    fn callback_is_required_without_an_implicit_identity_policy() {
        let input = syn::parse_quote!(
            struct Added {
                time: i64,
                value: u64,
            }
        );
        assert!(expand(input).unwrap_err().to_string().contains("id must be a closure"));
    }
    #[test]
    fn callback_is_invoked_without_hashing_or_automatic_hash_bounds() {
        let input = syn::parse_quote! {
            #[contime_event(id=|event| event.id)]
            struct Added<T> {time:i64,id:u128,payload:T}
        };
        let generated = expand(input).unwrap().to_string();
        assert!(generated.contains("fn (& Self) -> u128"));
        assert!(!generated.contains("identity ::"));
        assert!(!generated.contains("hash :: Hash"));
    }
    #[test]
    fn id_expressions_without_a_closure_are_rejected() {
        let input = syn::parse_quote! {
            #[contime_event(id=self.id)] struct Added {time:i64,id:u128}
        };
        assert!(expand(input).unwrap_err().to_string().contains("id must be a closure"));
    }
    #[test]
    fn hashing_is_an_explicit_optional_trait_implementation() {
        let input = syn::parse_quote! {
            #[contime_event(id=|event| event.id, hash=true)]
            struct Added<T> {time:i64,id:u128,payload:T}
        };
        let generated = expand(input).unwrap().to_string();
        assert!(generated.contains("hash :: Hash for Added"));
        assert!(generated.contains("T : :: core :: hash :: Hash"));
    }
}
