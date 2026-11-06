//! Independently generate selected timestamp and ID trait implementations.
use super::{
    events, field, identity, metadata, options,
    selector::{mapped, Selection},
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse_quote, DeriveInput};
pub(crate) fn event(input: DeriveInput) -> syn::Result<TokenStream> {
    let time_field = field::read(&input)?;
    let options = options::read(&input, false)?;
    let field_id = field::identity(&input)?;
    if field_id.is_some() && (options.id.is_some() || options.map.is_some()) {
        return Err(syn::Error::new_spanned(
            &input,
            "use either one ID field or a struct-level id selector; map requires the struct-level form",
        ));
    }
    if time_field.is_none() && field_id.is_none() && options.id.is_none() && options.events.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "mark a timestamp field with #[contime(time)] or supply #[contime(id = function)]",
        ));
    }
    let mut output = TokenStream::new();
    if let Some((member, time)) = time_field {
        let root = crate::expansion::root();
        let name = &input.ident;
        let mut generics = input.generics.clone();
        generics.make_where_clause().predicates.push(parse_quote!(#time: ::core::clone::Clone + ::core::cmp::PartialEq));
        let (ig, tg, wc) = generics.split_for_impl();
        output.extend(quote! {
            impl #ig #root::checkpoints::Event for #name #tg #wc {
                type Time = #time;
                fn time(&self) -> Self::Time {
                    ::core::clone::Clone::clone(&self.#member)
                }
            }
        });
    }
    let has_identity = field_id.is_some() || options.id.is_some();
    if let Some(member) = field_id {
        let function = syn::parse_quote!(|value| value.#member);
        output.extend(identity::event(&input, &function));
    } else if let Some(function) = &options.id {
        if let Some(map) = &options.map {
            let fields = Selection::read(function)?.render(quote!(self));
            let body = mapped(fields, Some(map));
            output.extend(identity::event_body(&input, body));
        } else {
            output.extend(identity::event(&input, function));
        }
    }
    if has_identity && !options.manual_duplicate_policy {
        let root = crate::expansion::root();
        let name = &input.ident;
        let mut generics = input.generics.clone();
        generics.make_where_clause().predicates.push(parse_quote!(Self: #root::EventId));
        let (ig, tg, wc) = generics.split_for_impl();
        output.extend(quote! {
            impl #ig #root::EventDuplicatePolicy for #name #tg #wc {
                type Context = ();
                const TRACK_ADMISSIONS: bool = false;
                const CUSTOM_POLICY: bool = false;
                fn create_context() -> Self::Context {}
            }
        });
    }
    Ok(output)
}
pub(crate) fn snapshot(input: DeriveInput) -> syn::Result<TokenStream> {
    let time_field = field::read(&input)?;
    let mut options = options::read(&input, true)?;
    if let Some(member) = field::identity(&input)? {
        if options.id.is_some() || options.map.is_some() {
            return Err(syn::Error::new_spanned(
                &input,
                "use either one ID field or a struct-level id selector; map requires the struct-level form",
            ));
        }
        // Normalize shorthand once so identity, routing and creation share exactly
        // the same selection rules as the explicit struct-level form.
        options.id = Some(syn::parse_quote!(|value| value.#member));
    }
    if time_field.is_none() && options.id.is_none() && options.events.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "mark a timestamp field with #[contime(time)] or supply #[contime(id = function)] or an event registration",
        ));
    }
    let mut output = TokenStream::new();
    if let Some((member, time)) = time_field {
        let root = crate::expansion::root();
        let name = &input.ident;
        let mut generics = input.generics.clone();
        let predicates = &mut generics.make_where_clause().predicates;
        predicates.push(parse_quote!(Self: ::core::clone::Clone));
        predicates.push(parse_quote!(#time: ::core::clone::Clone + ::core::default::Default + ::core::cmp::Ord));
        let (ig, tg, wc) = generics.split_for_impl();
        output.extend(quote! {
            impl #ig #root::checkpoints::Snapshot for #name #tg #wc {
                type Time = #time;
                fn time(&self) -> &Self::Time { &self.#member }
                fn set_time(&mut self, time: Self::Time) { self.#member = time; }
            }
        });
    }
    if let Some(function) = &options.id {
        if let Some(map) = &options.map {
            let fields = Selection::read(function)?.render(quote!(self));
            output.extend(identity::snapshot_body(&input, mapped(fields, Some(map))));
        } else {
            output.extend(identity::snapshot(&input, function));
        }
    }
    output.extend(events::expand(&input, &options)?);
    output.extend(metadata::expand(&input, &options)?);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helpers_require_at_least_one_explicit_request() {
        let input = syn::parse_quote!(
            struct S {
                at: i64,
            }
        );
        assert!(event(input).is_err());
    }
    #[test]
    fn time_only_helpers_generate_only_the_timestamp_trait() {
        for (expand, expected) in [(event as fn(DeriveInput) -> syn::Result<TokenStream>, "Event"), (snapshot, "Snapshot")] {
            let input = syn::parse_quote!(
                struct Value<T> {
                    payload: T,
                    #[contime(time)]
                    at: i64,
                }
            );
            let file: syn::File = syn::parse2(expand(input).unwrap()).unwrap();
            let implementations =
                file.items.iter().filter_map(|item| if let syn::Item::Impl(item) = item { Some(item) } else { None }).collect::<Vec<_>>();
            assert_eq!(implementations.len(), 1);
            let item = implementations[0];
            assert_eq!(item.trait_.as_ref().unwrap().1.segments.last().unwrap().ident, expected);
        }
    }
}
