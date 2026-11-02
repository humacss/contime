//! Copy explicit identity sources only when a snapshot is first initialized.
use super::selector::Selection;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Generics, Member, Type};

pub(super) fn expand(
    input: &DeriveInput,
    generics: &Generics,
    destination: &Selection,
    sources: &[(Type, Selection)],
) -> syn::Result<TokenStream> {
    let root = crate::expansion::root();
    let name = &input.ident;
    let mut generics = generics.clone();
    generics.make_where_clause().predicates.push(syn::parse_quote!(Self: ::core::default::Default));
    let Data::Struct(data) = &input.data else {
        unreachable!("field validation requires a struct");
    };
    for (member, _) in &destination.fields {
        let field = match member {
            Member::Named(name) => data.fields.iter().find(|f| f.ident.as_ref() == Some(name)),
            Member::Unnamed(index) => data.fields.iter().nth(index.index as usize),
        }
        .ok_or_else(|| syn::Error::new_spanned(member, "snapshot identity field does not exist"))?;
        let ty = &field.ty;
        generics.make_where_clause().predicates.push(syn::parse_quote!(#ty: ::core::clone::Clone));
    }
    let initializers = sources.iter().map(|(ty, source)| {
        let fields = destination.fields.iter().zip(&source.fields).map(|((to, _), (from, _))| {
            quote! {
                __snapshot.#to = ::core::clone::Clone::clone(&__event.#from);
            }
        });
        quote! {
            if let Some(__event) = __first.get::<#ty>() {
                let mut __snapshot = <Self as ::core::default::Default>::default();
                #(#fields)*
                return __snapshot;
            }
        }
    });
    let (ig, tg, wc) = generics.split_for_impl();
    Ok(quote! {
        impl #ig #root::SnapshotCreate for #name #tg #wc {
            fn create(_snapshot_id: u128, __first: #root::EventRef<'_>) -> Self {
                #(#initializers)*
                unreachable!("snapshot creation requires one of its registered event types")
            }
        }
    })
}
