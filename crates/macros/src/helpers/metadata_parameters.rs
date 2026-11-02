//! Lower snapshot-dependent event arguments to concrete alias parameters.
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{visit_mut::VisitMut, DeriveInput, GenericParam, Type};

pub(super) enum Argument {
    Type { name: syn::Ident },
    Const { name: syn::Ident, ty: Box<Type> },
    Lifetime { lifetime: syn::Lifetime },
}
impl Argument {
    pub fn declaration(&self) -> TokenStream {
        match self {
            Self::Type { name, .. } => quote!(#name:?Sized),
            Self::Const { name, ty, .. } => quote!(const #name:#ty),
            Self::Lifetime { lifetime } => quote!(#lifetime),
        }
    }
    pub fn local(&self) -> TokenStream {
        match self {
            Self::Type { name, .. } | Self::Const { name, .. } => quote!(#name),
            Self::Lifetime { lifetime } => quote!(#lifetime),
        }
    }
    pub fn binding(&self, definition: &syn::Ident) -> TokenStream {
        match self {
            Self::Type { name, .. } => quote!(<$snapshot as $($prefix)*::#definition>::#name),
            Self::Const { name, .. } => quote!({<$snapshot as $($prefix)*::#definition>::#name}),
            // Aggregate event lanes use Any and therefore require static data.
            Self::Lifetime { .. } => quote!('static),
        }
    }
}
pub(super) struct Lower<'a> {
    input: &'a DeriveInput,
    pub arguments: Vec<Argument>,
    pub declarations: Vec<TokenStream>,
    pub definitions: Vec<TokenStream>,
}
impl<'a> Lower<'a> {
    pub fn new(input: &'a DeriveInput) -> Self {
        Self { input, arguments: Vec::new(), declarations: Vec::new(), definitions: Vec::new() }
    }
    pub fn event(&mut self, mut ty: Type) -> (Type, std::ops::Range<usize>) {
        let start = self.arguments.len();
        self.visit_type_mut(&mut ty);
        (ty, start..self.arguments.len())
    }
    fn const_parameter(&self, name: &syn::Ident) -> Option<Type> {
        self.input.generics.params.iter().find_map(|p| match p {
            GenericParam::Const(p) if p.ident == *name => Some(p.ty.clone()),
            _ => None,
        })
    }
    fn type_parameter(&self, name: &syn::Ident) -> bool {
        self.input.generics.params.iter().any(|p| matches!(p,GenericParam::Type(p) if p.ident==*name))
    }
    fn add_const(&mut self, source: syn::Ident, ty: Type) -> syn::Ident {
        let name = format_ident!("__Const{}", self.arguments.len());
        let declaration_type = super::metadata_paths::at_definition(self.input, ty.clone());
        self.declarations.push(quote!(const #name:#declaration_type;));
        self.definitions.push(quote!(const #name:#ty=#source;));
        self.arguments.push(Argument::Const { name: name.clone(), ty: Box::new(ty) });
        name
    }
}
impl VisitMut for Lower<'_> {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if let Type::Path(path) = ty {
            if path.qself.is_none() && path.path.segments.len() == 1 {
                let source = &path.path.segments[0].ident;
                if let Some(param_type) = self.const_parameter(source) {
                    let name = self.add_const(source.clone(), param_type);
                    *ty = syn::parse_quote!(#name);
                    return;
                }
            }
            // Keep a projection such as T::Item intact inside the original
            // snapshot's impl, where all its bounds and predicates are available.
            let projected = path.qself.is_some() || path.path.segments.first().is_some_and(|s| self.type_parameter(&s.ident));
            if projected {
                let name = format_ident!("__Type{}", self.arguments.len());
                let source = ty.clone();
                self.declarations.push(quote!(type #name:?Sized;));
                self.definitions.push(quote!(type #name=#source;));
                self.arguments.push(Argument::Type { name: name.clone() });
                *ty = syn::parse_quote!(#name);
                return;
            }
        }
        syn::visit_mut::visit_type_mut(self, ty);
    }
    fn visit_expr_path_mut(&mut self, path: &mut syn::ExprPath) {
        if path.qself.is_none() && path.path.segments.len() == 1 {
            let source = &path.path.segments[0].ident;
            if let Some(ty) = self.const_parameter(source) {
                let name = self.add_const(source.clone(), ty);
                path.path = syn::parse_quote!(#name);
                return;
            }
        }
        syn::visit_mut::visit_expr_path_mut(self, path);
    }
    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
        if self.input.generics.params.iter().any(|p| matches!(p,GenericParam::Lifetime(p) if p.lifetime==*lifetime)) {
            let name = syn::Lifetime::new(&format!("'__life{}", self.arguments.len()), lifetime.ident.span());
            self.arguments.push(Argument::Lifetime { lifetime: name.clone() });
            *lifetime = name;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn associated_payloads_are_evaluated_with_the_snapshot_bounds() {
        let input = syn::parse_quote!(
            struct State<T: Family>(T);
        );
        let mut lower = Lower::new(&input);
        let (event, range) = lower.event(syn::parse_quote!(Changed<T::Item>));
        assert_eq!(quote!(#event).to_string(), quote!(Changed<__Type0>).to_string());
        assert_eq!(range.len(), 1);
        let definitions = &lower.definitions;
        assert!(quote!(#(#definitions)*).to_string().contains("T :: Item"));
    }
}
