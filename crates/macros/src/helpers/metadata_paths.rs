//! Rebase relative source paths without interpreting type/const argument names.
use syn::{visit_mut::VisitMut, DeriveInput, Type};
struct Relative {
    levels: usize,
}
impl VisitMut for Relative {
    fn visit_path_mut(&mut self, path: &mut syn::Path) {
        syn::visit_mut::visit_path_mut(self, path);
        if path.leading_colon.is_some() {
            return;
        }
        let Some(first) = path.segments.first_mut() else {
            return;
        };
        let self_path = first.ident == "self";
        if self_path {
            path.segments = std::mem::take(&mut path.segments).into_iter().skip(1).collect();
        }
        if self_path || path.segments.first().is_some_and(|s| s.ident == "super") {
            for _ in 0..self.levels {
                path.segments.insert(0, syn::parse_quote!(super));
            }
        }
    }
}
pub(super) fn source_type(mut ty: Type, levels: usize) -> Type {
    Relative { levels }.visit_type_mut(&mut ty);
    ty
}
pub(super) fn source_generics(mut generics: syn::Generics, levels: usize) -> syn::Generics {
    Relative { levels }.visit_generics_mut(&mut generics);
    generics
}
pub(super) fn at_definition(_: &DeriveInput, ty: Type) -> Type {
    source_type(ty, 1)
}
#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    #[test]
    fn relative_arguments_rebase_but_bare_type_or_const_arguments_remain_in_scope() {
        let ty = source_type(syn::parse_quote!(Event<super::Data,self::Other,SIZE>), 2);
        assert_eq!(quote!(#ty).to_string(), quote!(Event<super::super::super::Data,super::super::Other,SIZE>).to_string());
    }
}
