//! Validate the time-field marker without imposing a field name or time type.
use syn::{Data, DeriveInput, Index, Member, Token, Type};
pub(super) fn read(input: &DeriveInput) -> syn::Result<Option<(Member, Type)>> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(input, "timestamp helpers require a struct"));
    };
    let mut selected = None;
    for (index, field) in data.fields.iter().enumerate() {
        let mut marked = false;
        for attr in field.attrs.iter().filter(|a| a.path().is_ident("contime")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("id") {
                    return Ok(());
                }
                if !meta.path.is_ident("time") {
                    return Err(meta.error("expected #[contime(time)] or the field marker #[contime(id)]"));
                }
                if meta.input.peek(Token![=]) || meta.input.peek(syn::token::Paren) {
                    return Err(meta.error("time is a marker; use #[contime(time)] without a value"));
                }
                if marked {
                    return Err(meta.error("duplicate time marker on this field"));
                }
                marked = true;
                Ok(())
            })?;
        }
        if marked {
            if selected.is_some() {
                return Err(syn::Error::new_spanned(field, "only one field may be marked #[contime(time)]"));
            }
            let member = field.ident.clone().map(Member::Named).unwrap_or_else(|| Member::Unnamed(Index::from(index)));
            selected = Some((member, field.ty.clone()));
        }
    }
    Ok(selected)
}
/// The single-field shorthand never combines with a struct-level selector or map.
pub(super) fn identity(input: &DeriveInput) -> syn::Result<Option<Member>> {
    let Data::Struct(data) = &input.data else {
        return Ok(None);
    };
    let mut selected = None;
    for (index, field) in data.fields.iter().enumerate() {
        for attr in field.attrs.iter().filter(|a| a.path().is_ident("contime")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("id") {
                    if meta.input.peek(Token![=]) || meta.input.peek(syn::token::Paren) {
                        return Err(meta.error("id is a marker; use #[contime(id)] without a value"));
                    }
                    if selected.is_some() {
                        return Err(meta.error("only one field may be marked #[contime(id)]"));
                    }
                    selected = Some(field.ident.clone().map(Member::Named).unwrap_or_else(|| Member::Unnamed(Index::from(index))));
                }
                Ok(())
            })?;
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_markers_are_rejected() {
        for input in [
            syn::parse_quote!(
                struct S {
                    #[contime(time)]
                    a: i64,
                    #[contime(time)]
                    b: i64,
                }
            ),
            syn::parse_quote!(
                struct S {
                    #[contime(time, time)]
                    a: i64,
                }
            ),
            syn::parse_quote!(
                struct S {
                    #[contime(unknown)]
                    a: i64,
                }
            ),
            syn::parse_quote!(
                struct S {
                    #[contime(time = true)]
                    a: i64,
                }
            ),
            syn::parse_quote!(
                enum S {
                    A,
                }
            ),
        ] {
            assert!(read(&input).is_err());
        }
    }
    #[test]
    fn named_and_tuple_fields_are_selected() {
        let named = syn::parse_quote!(
            struct S {
                other: String,
                #[contime(time)]
                at: i64,
            }
        );
        assert!(matches!(read(&named).unwrap().unwrap().0,Member::Named(n)if n=="at"));
        let tuple = syn::parse_quote!(
            struct S(String, #[contime(time)] i64);
        );
        assert!(matches!(read(&tuple).unwrap().unwrap().0,Member::Unnamed(i)if i.index==1));
    }
}
