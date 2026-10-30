//! Single- and multi-destination routing from event fields to snapshot identities.
use super::definition::array;
use crate::attributes::Route;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{visit_mut::VisitMut, DeriveInput, Expr};
struct ReplaceSelf;
impl VisitMut for ReplaceSelf {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if let Expr::Path(path) = expr {
            if path.path.is_ident("self") {
                *expr = syn::parse_quote!(__event);
                return;
            }
        }
        syn::visit_mut::visit_expr_mut(self, expr);
    }
}
fn event_expr(mut expr: Expr) -> Expr {
    ReplaceSelf.visit_expr_mut(&mut expr);
    expr
}

pub(super) fn expand(
    input: &DeriveInput,
    routes: &[Route],
    id_fields: &[syn::Ident],
    generics: &syn::Generics,
) -> syn::Result<TokenStream> {
    let root = crate::expansion::root();
    let name = &input.ident;
    let (ig, tg, wc) = generics.split_for_impl();
    let mut implementations = quote!();
    for route in routes {
        let event = &route.event;
        let target = route.target.clone().unwrap_or_else(|| syn::parse_quote!([#(self.#id_fields),*]));
        let (setup, values) = if let Expr::Call(call) = &target {
            if let Expr::Path(path) = &*call.func {
                if path.path.is_ident("each") && call.args.len() == 2 {
                    let source = event_expr(call.args[0].clone());
                    let closure = match &call.args[1] {
                        Expr::Closure(c) => c,
                        _ => return Err(syn::Error::new_spanned(&call.args[1], "each requires a closure")),
                    };
                    let pat = &closure.inputs;
                    let values = array(&event_expr((*closure.body).clone()))?;
                    (Some(quote!(for #pat in &(#source))), values)
                } else {
                    return Err(syn::Error::new_spanned(target, "expected [fields] or each(collection, |item| [fields])"));
                }
            } else {
                return Err(syn::Error::new_spanned(target, "invalid route"));
            }
        } else {
            (None, array(&event_expr(target))?)
        };
        if values.len() != id_fields.len() {
            return Err(syn::Error::new_spanned(event, "route identity must have the same number of components as snapshot identity"));
        }
        let route_id = quote!(#root::module::identity::<Self,_>(&(#(&(#values),)*)));
        let emit = quote!(emit(#route_id););
        let emit = if let Some(setup) = &setup { quote!(#setup {#emit}) } else { emit };
        let create = quote!(if snapshot_id==#route_id { let mut snapshot=Self::default(); #(snapshot.#id_fields=(#values).clone();)* return Some(snapshot); });
        let create = if let Some(setup) = &setup { quote!(#setup {#create}) } else { create };
        implementations.extend(quote! {impl #ig #root::module::Route<#event> for #name #tg #wc {
            fn snapshot_ids(__event:&#event,emit:&mut impl FnMut(u128)) {#emit}
            fn create(snapshot_id:u128,__event:&#event)->Option<Self> {#create None}
        }});
    }
    Ok(implementations)
}
