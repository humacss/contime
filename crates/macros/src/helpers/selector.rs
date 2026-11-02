//! Declarative field selection, without inspecting arbitrary user computation.
use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{Expr, Member, Pat};

#[derive(Clone)]
pub(super) struct Selection {
    pub fields: Vec<(Member, bool)>,
    pub tuple: bool,
}
impl Selection {
    pub fn read(expr: &Expr) -> syn::Result<Self> {
        let Expr::Closure(closure) = expr else {
            return Err(syn::Error::new_spanned(
                expr,
                "select identity fields with |value| value.field or |value| (value.first, value.second)",
            ));
        };
        if closure.inputs.len() != 1 || closure.asyncness.is_some() {
            return Err(syn::Error::new_spanned(closure, "an identity selector takes one untyped value argument"));
        }
        let Some(Pat::Ident(argument)) = closure.inputs.first() else {
            return Err(syn::Error::new_spanned(closure, "an identity selector takes one untyped value argument"));
        };
        let (items, tuple): (Vec<&Expr>, _) = match &*closure.body {
            Expr::Tuple(items) if !items.elems.is_empty() => (items.elems.iter().collect(), true),
            other => (vec![other], false),
        };
        let mut fields = Vec::new();
        for item in items {
            let (item, borrowed) = if let Expr::Reference(reference) = item {
                if reference.mutability.is_some() {
                    return Err(syn::Error::new_spanned(reference, "identity fields can only be borrowed immutably"));
                }
                (&*reference.expr, true)
            } else {
                (item, false)
            };
            let Expr::Field(field) = item else {
                return Err(selector_error(item));
            };
            let Expr::Path(base) = &*field.base else {
                return Err(selector_error(item));
            };
            if !base.path.is_ident(&argument.ident) {
                return Err(selector_error(item));
            }
            fields.push((field.member.clone(), borrowed));
        }
        Ok(Self { fields, tuple })
    }
    pub fn render(&self, receiver: TokenStream) -> TokenStream {
        let parts =
            self.fields.iter().map(|(member, borrowed)| if *borrowed { quote!(&#receiver.#member) } else { quote!(#receiver.#member) });
        if self.tuple {
            quote!((#(#parts,)*))
        } else {
            parts.into_iter().next().unwrap()
        }
    }
    pub fn same_shape(&self, other: &Self) -> bool {
        self.tuple == other.tuple && self.fields.len() == other.fields.len()
    }
    pub fn unique(&self) -> bool {
        let mut fields = std::collections::HashSet::new();
        self.fields.iter().all(|(field, _)| fields.insert(field.to_token_stream().to_string()))
    }
}
fn selector_error(expr: &Expr) -> syn::Error {
    syn::Error::new_spanned(expr, "select direct fields or a tuple of fields; put identity calculations in map")
}
/// Keeping the input argument first lets Rust infer tuple/reference map parameters.
pub(super) fn mapped(fields: TokenStream, map: Option<&Expr>) -> TokenStream {
    match map {
        None => fields,
        Some(map) => quote!({
            fn __contime_map<Fields>(fields: Fields, map: impl FnOnce(Fields) -> u128) -> u128 { map(fields) }
            __contime_map(#fields, #map)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selectors_preserve_tuple_shape_and_borrowed_fields() {
        let selector = Selection::read(&syn::parse_quote!(|value| (&value.owner, value.slot))).unwrap();
        assert!(selector.tuple);
        assert!(selector.fields[0].1);
        assert!(!selector.fields[1].1);
        assert!(selector.unique());
        assert_eq!(selector.render(quote!(event)).to_string(), quote!((&event.owner, event.slot,)).to_string());
    }
    #[test]
    fn calculations_and_hidden_field_selection_are_rejected() {
        for expr in [quote!(|v| calculate(v)), quote!(|v| v.id + 1), quote!(|v| v.inner.id), quote!(|a, b| a.id), quote!(|v| ())] {
            assert!(Selection::read(&syn::parse2(expr).unwrap()).is_err());
        }
    }
}
