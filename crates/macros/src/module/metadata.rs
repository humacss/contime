//! Definition-site aliases and the cross-crate metadata callback protocol.
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    Ident, Path, Token, Type, Visibility,
};
pub(crate) fn sibling(path: &Path, prefix: &str) -> Path {
    let mut path = path.clone();
    let leaf = path.segments.last_mut().unwrap();
    leaf.ident = format_ident!("{}{}", prefix, leaf.ident);
    leaf.arguments = syn::PathArguments::None;
    path
}
pub(crate) fn definition_path(path: &Path) -> Path {
    if path.leading_colon.is_some() || path.segments.first().is_some_and(|segment| segment.ident == "crate") {
        return path.clone();
    }
    let mut path = path.clone();
    if path.segments.first().is_some_and(|segment| segment.ident == "self") {
        path.segments.first_mut().unwrap().ident = Ident::new("super", path.segments.first().unwrap().ident.span());
        return path;
    }
    if path.segments.len() == 1 || path.segments.first().is_some_and(|segment| segment.ident == "super") {
        path.segments.insert(0, syn::parse_quote!(super));
    }
    path
}
fn list<T: Parse>(input: ParseStream<'_>) -> syn::Result<Vec<T>> {
    let content;
    syn::bracketed!(content in input);
    Ok(content.parse_terminated(T::parse, Token![,])?.into_iter().collect())
}
#[derive(Clone)]
pub(super) struct Lane {
    pub(super) name: Ident,
    pub(super) path: Path,
    pub(super) subset: Option<Vec<Lane>>,
}
impl Parse for Lane {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![=]>()?;
        let path = input.parse()?;
        let subset = if input.peek(Ident) {
            let tag: Ident = input.parse()?;
            if tag != "only" {
                return Err(syn::Error::new_spanned(tag, "expected only"));
            }
            Some(list(input)?)
        } else {
            None
        };
        Ok(Self { name, path, subset })
    }
}
pub(super) struct Pending {
    pub(super) callback: Path,
    pub(super) prefix: Path,
    pub(super) ancestors: Vec<Ident>,
    pub(super) snapshot: Option<Lane>,
}
impl Parse for Pending {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let content;
        syn::parenthesized!(content in input);
        Ok(Self {
            callback: content.parse()?,
            prefix: {
                content.parse::<Token![,]>()?;
                content.parse()?
            },
            ancestors: {
                content.parse::<Token![,]>()?;
                list(&content)?
            },
            snapshot: if content.peek(Token![;]) {
                content.parse::<Token![;]>()?;
                Some(content.parse()?)
            } else {
                None
            },
        })
    }
}
pub struct State {
    pub(super) vis: Visibility,
    pub(super) name: Ident,
    pub(super) time: Type,
    pub(super) events: Vec<Lane>,
    pub(super) snapshots: Vec<Lane>,
    pub(super) pending: Vec<Pending>,
    pub(super) seen: Vec<Ident>,
    pub(super) current: Option<Current>,
}
pub(super) struct Current {
    pub(super) key: Ident,
    pub(super) ancestors: Vec<Ident>,
    pub(super) events: Vec<Lane>,
    pub(super) snapshots: Vec<Lane>,
    pub(super) pending: Vec<Pending>,
    pub(super) time: Type,
    pub(super) snapshot: bool,
    pub(super) subset: Option<Vec<Lane>>,
}
impl Parse for State {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let vis = input.parse()?;
        let name = input.parse()?;
        input.parse::<Token![;]>()?;
        let time = input.parse()?;
        input.parse::<Token![;]>()?;
        let events = list(input)?;
        let snapshots = list(input)?;
        let pending = list(input)?;
        let seen = list(input)?;
        let current = if input.is_empty() {
            None
        } else {
            Some(Current {
                key: input.parse()?,
                ancestors: list(input)?,
                time: input.parse()?,
                events: list(input)?,
                snapshots: list(input)?,
                pending: list(input)?,
                snapshot: if input.peek(Ident) {
                    let tag: Ident = input.parse()?;
                    if tag != "snapshot" {
                        return Err(syn::Error::new_spanned(tag, "expected snapshot"));
                    }
                    true
                } else {
                    false
                },
                subset: if input.peek(syn::token::Bracket) {
                    let selection;
                    syn::bracketed!(selection in input);
                    let tag: Ident = selection.parse()?;
                    if tag == "all" {
                        None
                    } else if tag == "only" {
                        Some(list(&selection)?)
                    } else {
                        return Err(syn::Error::new_spanned(tag, "expected all or only"));
                    }
                } else {
                    None
                },
            })
        };
        Ok(Self { vis, name, time, events, snapshots, pending, seen, current })
    }
}
pub(super) fn lane_tokens(lanes: &[Lane]) -> Vec<TokenStream> {
    lanes
        .iter()
        .map(|l| {
            let n = &l.name;
            let p = &l.path;
            if let Some(subset) = &l.subset {
                let events = lane_tokens(subset);
                quote!(#n=#p only [#(#events),*])
            } else {
                quote!(#n=#p)
            }
        })
        .collect()
}
pub(super) fn pending_tokens(pending: &[Pending]) -> Vec<TokenStream> {
    pending
        .iter()
        .map(|p| {
            let c = &p.callback;
            let r = &p.prefix;
            let a = &p.ancestors;
            if let Some(snapshot) = &p.snapshot {
                let snapshot = lane_tokens(std::slice::from_ref(snapshot));
                {
                    quote!((#c,#r,[#(#a),*];#(#snapshot)*))
                }
            } else {
                quote!((#c,#r,[#(#a),*]))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::derive;
    #[test]
    fn leaf_paths_require_no_definition_metadata_macro() {
        let input = syn::parse_quote!(
            #[contime_module(time=i64,events=[Added],snapshots=[State])]
            struct App;
        );
        let code = derive(input).unwrap().to_string();
        assert!(code.contains("pub type Event0"));
        assert!(!code.contains("@ contime_definition"));
        assert!(!code.contains("Added !"));
    }
    #[test]
    fn lane_metadata_round_trips_with_one_registration_name() {
        let state: State = syn::parse_quote!(App;i64;[Added=first::Added] [State=second::State] [] []);
        let events = lane_tokens(&state.events);
        let snapshots = lane_tokens(&state.snapshots);
        let parsed: State = syn::parse2(quote!(App;i64;[#(#events),*] [#(#snapshots),*] [] [])).unwrap();
        assert_eq!(parsed.events[0].name, "Added");
        assert_eq!(parsed.snapshots[0].name, "State");
        assert!(!quote!(#(#events)* #(#snapshots)*).into_iter().any(|t| matches!(t, proc_macro2::TokenTree::Literal(_))));
    }
    #[test]
    fn only_concrete_empty_structs_are_module_markers() {
        for input in [
            syn::parse_quote!(
                #[contime_module(time=i64)]
                struct App<T>(T);
            ),
            syn::parse_quote!(
                #[contime_module(time=i64)]
                enum App {}
            ),
        ] {
            assert!(derive(input).is_err());
        }
    }
}
