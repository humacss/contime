//! Borrowed event projection and the generic parameters needed by its enum.
use crate::attributes::{variant, Route};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{visit_mut::VisitMut, DeriveInput};

pub(super) struct BorrowedEvents {
    pub declaration: TokenStream,
    pub view_arms: TokenStream,
    pub event_bounds: Vec<TokenStream>,
}
pub(super) fn expand(input: &DeriveInput, routes: &[Route], event_name: &syn::Ident) -> syn::Result<BorrowedEvents> {
    let root = crate::expansion::root();
    let vis = &input.vis;
    let event_variants = routes.iter().map(|route| {
        let event = &route.event;
        let variant = variant(event);
        quote!(#[doc = "A borrowed registered event payload."] #variant(&'__event #event))
    });
    let view_arms = routes.iter().map(|route| {
        let event = &route.event;
        let variant = variant(event);
        quote!(if let Some(event)=<__I as #root::module::View<#event>>::get(event) {return Some(#event_name::#variant(event));})
    });
    let event_bounds = routes
        .iter()
        .map(|route| {
            let event = &route.event;
            quote!(#root::module::View<#event>)
        })
        .collect::<Vec<_>>();
    let mut used = UsedParameters::default();
    for route in routes {
        let mut path = route.event.clone();
        used.visit_path_mut(&mut path);
    }
    let mut event_generics = input.generics.clone();
    event_generics.params = event_generics.params.into_iter().filter(|parameter| used.names.contains(&parameter_name(parameter))).collect();
    let omitted = input
        .generics
        .params
        .iter()
        .map(parameter_name)
        .filter(|name| !used.names.contains(name))
        .collect::<std::collections::HashSet<_>>();
    for parameter in &event_generics.params {
        let mut names = UsedParameters::default();
        let mut parameter = parameter.clone();
        names.visit_generic_param_mut(&mut parameter);
        if !names.names.is_disjoint(&omitted) {
            return Err(syn::Error::new_spanned(
                parameter,
                "borrowed event bounds must not refer to snapshot parameters absent from its event types",
            ));
        }
    }
    if let Some(clause) = &mut event_generics.where_clause {
        clause.predicates = clause
            .predicates
            .clone()
            .into_iter()
            .filter(|predicate| {
                let mut visitor = UsedParameters::default();
                let mut predicate = predicate.clone();
                visitor.visit_where_predicate_mut(&mut predicate);
                visitor.names.is_disjoint(&omitted)
            })
            .collect();
    }
    event_generics.params.insert(0, syn::parse_quote!('__event));
    let event_where = &event_generics.where_clause;
    Ok(BorrowedEvents {
        declaration: quote! {
            #[doc = "Borrowed event payloads applied to this snapshot."]
            #vis enum #event_name #event_generics #event_where {#(#event_variants,)*}
        },
        view_arms: quote!(#(#view_arms)*),
        event_bounds,
    })
}
#[derive(Default)]
struct UsedParameters {
    names: std::collections::HashSet<String>,
}
impl VisitMut for UsedParameters {
    fn visit_ident_mut(&mut self, id: &mut syn::Ident) {
        self.names.insert(id.to_string());
    }
}
fn parameter_name(parameter: &syn::GenericParam) -> String {
    match parameter {
        syn::GenericParam::Type(p) => p.ident.to_string(),
        syn::GenericParam::Lifetime(p) => p.lifetime.ident.to_string(),
        syn::GenericParam::Const(p) => p.ident.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn borrowed_snapshot_enum_retains_generic_payload_parameter() {
        let input: DeriveInput = syn::parse_quote! {
            #[contime_snapshot(id=|owner| u128::from(*owner),events=[Added<T>],apply={for _ in batch.events {}})]
            struct Counter<T:Clone+Default> {time:i64,owner:u64,value:T}
        };
        let options = crate::attributes::Options::read(&input, false).unwrap();
        let name = syn::parse_quote!(CounterEvent);
        let generated: syn::File = syn::parse2(expand(&input, &options.events, &name).unwrap().declaration).unwrap();
        let events = generated
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Enum(item) if item.ident == "CounterEvent" => Some(item),
                _ => None,
            })
            .unwrap();
        assert_eq!(events.generics.params.len(), 2);
        let syn::Fields::Unnamed(fields) = &events.variants[0].fields else { panic!("borrowed payload tuple") };
        let syn::Type::Reference(reference) = &fields.unnamed[0].ty else { panic!("payload must be borrowed") };
        assert_eq!(quote!(#reference).to_string(), "& '__event Added < T >");
    }
}
