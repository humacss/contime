//! Timestamp access, automatic hashing and event identity.
use crate::attributes::{bounds, fields, time, Options};
use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let options = Options::read(&input, false)?;
    let root = crate::expansion::root();
    let name = &input.ident;
    let (time, time_ty) = time(&input, &options)?;
    let fields = fields(&input)?;
    let names = fields.iter().map(|(name, _)| name);
    let hash = if options.id.is_none() && options.hash.unwrap_or(true) {
        let gen = bounds(input.generics.clone(), &fields, quote!(::core::hash::Hash));
        let (ig, tg, wc) = gen.split_for_impl();
        quote! { impl #ig ::core::hash::Hash for #name #tg #wc { fn hash<__H: ::core::hash::Hasher>(&self,state:&mut __H) { #(::core::hash::Hash::hash(&self.#names,state);)* } } }
    } else {
        quote!()
    };
    let whole_identity = options.id.is_none();
    let id = options.id.unwrap_or_else(|| syn::parse_quote!(self));
    let mut event_generics = input.generics.clone();
    event_generics.make_where_clause().predicates.push(syn::parse_quote!(#time_ty: ::core::clone::Clone + ::core::cmp::PartialEq));
    let (ig, tg, wc) = event_generics.split_for_impl();
    let mut identity_generics = event_generics.clone();
    if whole_identity {
        identity_generics.make_where_clause().predicates.push(syn::parse_quote!(Self: ::core::hash::Hash));
    }
    let (iig, itg, iwc) = identity_generics.split_for_impl();
    let metadata = crate::registration::metadata(name);
    Ok(quote! {
        #metadata
        #hash
        impl #ig #root::checkpoints::Event for #name #tg #wc { type Time=#time_ty; fn time(&self)->Self::Time { #time.clone() } }
        impl #iig #name #itg #iwc { #[doc = "Returns this event's stable, namespaced identity."] pub fn event_id(&self)->u128 { <Self as #root::module::Event>::event_id(self) } }
        impl #iig #root::module::Event for #name #itg #iwc { fn event_id(&self)->u128 { #root::module::identity::<Self,_>(&#id) } }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_event_identity_adds_hash_bounds_but_custom_identity_does_not() {
        let whole = expand(syn::parse_quote!(
            struct Added<T> {
                time: i64,
                payload: T,
            }
        ))
        .unwrap();
        let whole: syn::File = syn::parse2(whole).unwrap();
        let hash = whole
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Impl(item) if item.trait_.as_ref().is_some_and(|(_, path, _)| path.segments.last().unwrap().ident == "Hash") => {
                    Some(item)
                }
                _ => None,
            })
            .unwrap();
        assert!(hash
            .generics
            .where_clause
            .as_ref()
            .unwrap()
            .predicates
            .iter()
            .any(|predicate| quote!(#predicate).to_string().contains("T : :: core :: hash :: Hash")));
        let custom = expand(syn::parse_quote!(
            #[contime_event(id=self.id)]
            struct Added<T> {
                time: i64,
                id: u64,
                payload: T,
            }
        ))
        .unwrap();
        let custom: syn::File = syn::parse2(custom).unwrap();
        assert!(!custom.items.iter().any(|item|matches!(item,syn::Item::Impl(item) if item.trait_.as_ref().is_some_and(|(_,path,_)|path.segments.last().unwrap().ident=="Hash"))));
    }
}
