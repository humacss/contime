//! Parse independent identity options and default or customized event registrations.
use quote::ToTokens;
use syn::{bracketed, punctuated::Punctuated, DeriveInput, Expr, Token, Type};

#[derive(Default)]
pub(super) struct Options {
    pub id: Option<Expr>,
    pub map: Option<Expr>,
    pub events: Vec<Registration>,
}
pub(super) struct Registration {
    pub ty: Type,
    pub id: Option<Expr>,
    pub map: Option<Expr>,
}
impl Registration {
    // Retain the earlier explicit emitter form for advanced/manual initialization.
    pub fn emitter(&self) -> Option<&Expr> {
        self.id.as_ref().filter(|id| match id {
            Expr::Path(_) => true,
            Expr::Closure(c) => c.inputs.len() == 2,
            _ => false,
        })
    }
}
pub(super) fn read(input: &DeriveInput, snapshot: bool) -> syn::Result<Options> {
    let mut result = Options::default();
    let mut seen = std::collections::HashSet::new();
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("contime")) {
        let mut event = None;
        let mut events = None;
        let mut id = None;
        let mut map = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("event") || meta.path.is_ident("events") {
                if !snapshot {
                    return Err(meta.error("event registrations belong on ContimeSnapshot"));
                }
                if event.is_some() || events.is_some() {
                    return Err(meta.error("use one event or one events list per attribute"));
                }
                if meta.path.is_ident("event") {
                    event = Some(meta.value()?.parse::<Type>()?);
                } else {
                    let content;
                    let value = meta.value()?;
                    bracketed!(content in value);
                    let list = Punctuated::<Type, Token![,]>::parse_terminated(&content)?;
                    if list.is_empty() {
                        return Err(meta.error("events requires at least one event type"));
                    }
                    events = Some(list);
                }
            } else if meta.path.is_ident("id") || meta.path.is_ident("map") {
                let slot = if meta.path.is_ident("id") { &mut id } else { &mut map };
                if slot.is_some() {
                    return Err(meta.error("duplicate identity option"));
                }
                let value: Expr = meta.value()?.parse()?;
                if !matches!(value, Expr::Path(_) | Expr::Closure(_)) {
                    return Err(syn::Error::new_spanned(value, "id and map must be function paths or noncapturing closures"));
                }
                *slot = Some(value);
            } else if meta.path.is_ident("time") {
                return Err(meta.error("put #[contime(time)] on the timestamp field, not the struct"));
            } else {
                return Err(meta.error("expected id, map, event or events"));
            }
            Ok(())
        })?;
        let mut add = |ty: Type, id, map| -> syn::Result<()> {
            if !seen.insert(ty.to_token_stream().to_string()) {
                return Err(syn::Error::new_spanned(&ty, "duplicate event registration"));
            }
            result.events.push(Registration { ty, id, map });
            Ok(())
        };
        if let Some(types) = events {
            if id.is_some() || map.is_some() {
                return Err(syn::Error::new_spanned(attr, "customize events individually with event = Type"));
            }
            for ty in types {
                add(ty, None, None)?;
            }
        } else if let Some(ty) = event {
            add(ty, id, map)?;
        } else {
            if let Some(id) = id {
                if result.id.replace(id).is_some() {
                    return Err(syn::Error::new_spanned(attr, "duplicate id selector"));
                }
            }
            if let Some(map) = map {
                if result.map.replace(map).is_some() {
                    return Err(syn::Error::new_spanned(attr, "duplicate ID map"));
                }
            }
        }
    }
    if result.map.is_some() && result.id.is_none() {
        return Err(syn::Error::new_spanned(input, "map requires a struct-level id selector"));
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lists_and_overrides_are_independent_of_parameter_order() {
        let input = syn::parse_quote!(
            #[contime(map = hash, id = |v| v.id)]
            #[contime(events = [Added, Removed,])]
            #[contime(id = |v| v.destination, event = Forwarded)]
            struct State;
        );
        let options = read(&input, true).unwrap();
        assert!(options.map.is_some());
        assert_eq!(options.events.len(), 3);
        assert!(read(&input, false).is_err());
    }
    #[test]
    fn ambiguous_or_incomplete_options_are_rejected() {
        for attrs in [
            quote::quote!(#[contime(map = hash)]),
            quote::quote!(#[contime(events = [])]),
            quote::quote!(#[contime(events = [Added], id = |v| v.id)]),
            quote::quote!(#[contime(events = [Added])] #[contime(event = Added)]),
            quote::quote!(#[contime(id = f, id = g)]),
        ] {
            assert!(read(&syn::parse2(quote::quote!(#attrs struct State;)).unwrap(), true).is_err());
        }
    }
}
