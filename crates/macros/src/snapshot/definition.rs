//! Validate snapshot identity fields and generate automatic Clone/Default implementations.
use crate::attributes::{bounds, fields, literal_self, time, Options};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Expr, Member, Type};

pub(super) struct Definition {
    pub time: Expr,
    pub time_ty: Type,
    pub id_fields: Vec<syn::Ident>,
    pub id_types: Vec<Type>,
    pub auto: TokenStream,
    pub generics: syn::Generics,
}
pub(super) fn array(expr: &Expr) -> syn::Result<Vec<Expr>> {
    match expr {
        Expr::Array(a) => Ok(a.elems.iter().cloned().collect()),
        _ => Err(syn::Error::new_spanned(expr, "identity must be a list such as [self.owner]")),
    }
}
pub(super) fn read(input: &DeriveInput, options: &Options) -> syn::Result<Definition> {
    let name = &input.ident;
    let (time, time_ty) = time(input, options)?;
    let fields = fields(input)?;
    let id_expr = options.id.as_ref().ok_or_else(|| syn::Error::new_spanned(name, "snapshot requires id = [self.field, ...]"))?;
    let id_fields = array(id_expr)?
        .into_iter()
        .map(|expr| match expr {
            Expr::Field(f) if literal_self(&f.base) => match f.member {
                Member::Named(id) => Ok(id),
                _ => Err(syn::Error::new_spanned(f, "identity must name fields")),
            },
            other => Err(syn::Error::new_spanned(other, "identity must name snapshot fields")),
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let id_types = id_fields
        .iter()
        .map(|id| {
            fields
                .iter()
                .find(|(n, _)| *n == id)
                .map(|(_, ty)| (*ty).clone())
                .ok_or_else(|| syn::Error::new_spanned(id, "identity field not found"))
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let mut auto = quote!();
    if options.clone.unwrap_or(true) {
        let gen = bounds(input.generics.clone(), &fields, quote!(::core::clone::Clone));
        let (ig, tg, wc) = gen.split_for_impl();
        let names = fields.iter().map(|(n, _)| n);
        auto.extend(
            quote! {impl #ig ::core::clone::Clone for #name #tg #wc {fn clone(&self)->Self {Self {#(#names:self.#names.clone(),)*}}}},
        );
    }
    if options.default.unwrap_or(true) {
        let gen = bounds(input.generics.clone(), &fields, quote!(::core::default::Default));
        let (ig, tg, wc) = gen.split_for_impl();
        let names = fields.iter().map(|(n, _)| n);
        auto.extend(quote!{impl #ig ::core::default::Default for #name #tg #wc {fn default()->Self {Self {#(#names: ::core::default::Default::default(),)*}}}});
    }
    let mut snapshot_generics = input.generics.clone();
    snapshot_generics.make_where_clause().predicates.push(syn::parse_quote!(Self: ::core::clone::Clone + ::core::default::Default));
    snapshot_generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(#time_ty: ::core::clone::Clone + ::core::default::Default + ::core::cmp::Ord));
    for ty in &id_types {
        snapshot_generics.make_where_clause().predicates.push(syn::parse_quote!(#ty: ::core::clone::Clone + ::core::hash::Hash));
    }
    Ok(Definition { time, time_ty, id_fields, id_types, auto, generics: snapshot_generics })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_requires_literal_self_fields() {
        let input: DeriveInput = syn::parse_quote!(
            #[contime_snapshot(id=[other.owner],events=[Added],apply={})]
            struct Counter {
                time: i64,
                owner: u64,
            }
        );
        let options = Options::read(&input, false).unwrap();
        assert!(read(&input, &options).err().unwrap().to_string().contains("identity must name snapshot fields"));
    }
}
