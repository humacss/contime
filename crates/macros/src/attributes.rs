//! Attribute grammar and shared inspection of named struct fields.
use proc_macro2::TokenStream;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Data, DeriveInput, Expr, Fields, Ident, Member, Path, Token, Type,
};

#[derive(Default)]
pub struct Options {
    pub time: Option<Expr>,
    pub id: Option<Expr>,
    pub hash: Option<bool>,
    pub clone: Option<bool>,
    pub default: Option<bool>,
    pub apply: Option<syn::Block>,
    pub create: Option<Expr>,
    pub events: Vec<Route>,
    pub snapshots: Vec<Path>,
    pub modules: Vec<Path>,
    pub time_type: Option<Type>,
}
pub struct Route {
    pub event: Path,
    pub target: Option<Expr>,
}
impl Parse for Route {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            event: input.parse()?,
            target: if input.peek(Token![=>]) {
                input.parse::<Token![=>]>()?;
                Some(input.parse()?)
            } else {
                None
            },
        })
    }
}
impl Options {
    pub fn read(input: &syn::DeriveInput, module: bool) -> syn::Result<Self> {
        let mut out = Self::default();
        for attr in &input.attrs {
            if !(attr.path().is_ident("contime")
                || attr.path().is_ident("contime_event")
                || attr.path().is_ident("contime_snapshot")
                || attr.path().is_ident("contime_module"))
            {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                let key = meta.path.get_ident().ok_or_else(|| meta.error("expected option name"))?.to_string();
                let value = meta.value()?;
                match key.as_str() {
                    "time" if module => out.time_type = Some(value.parse()?),
                    "time" => out.time = Some(value.parse()?),
                    "id" => out.id = Some(value.parse()?),
                    "hash" => out.hash = Some(value.parse::<syn::LitBool>()?.value),
                    "clone" => out.clone = Some(value.parse::<syn::LitBool>()?.value),
                    "default" => out.default = Some(value.parse::<syn::LitBool>()?.value),
                    "apply" => out.apply = Some(value.parse()?),
                    "create" if !module => out.create = Some(value.parse()?),
                    "events" => {
                        let content;
                        syn::bracketed!(content in value);
                        out.events = Punctuated::<Route, Token![,]>::parse_terminated(&content)?.into_iter().collect();
                    }
                    "snapshots" | "modules" => {
                        let content;
                        syn::bracketed!(content in value);
                        let paths = Punctuated::<Path, Token![,]>::parse_terminated(&content)?.into_iter().collect();
                        if key == "snapshots" {
                            out.snapshots = paths;
                        } else {
                            out.modules = paths;
                        }
                    }
                    _ => return Err(meta.error("unknown ConTime option")),
                }
                Ok(())
            })?;
        }
        Ok(out)
    }
}
pub fn variant(path: &Path) -> Ident {
    path.segments.last().unwrap().ident.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_routing_and_apply() {
        let input = syn::parse_quote! {
            #[contime(id = |owner| u128::from(*owner), events = [Added, Other => [self.other], Multi => each(self.targets, |item| [item])], apply = { for event in batch.events {} })]
            struct Counter { owner: u64, time: i64 }
        };
        let options = Options::read(&input, false).unwrap();
        assert_eq!(options.events.len(), 3);
        assert!(options.events[0].target.is_none());
        assert!(options.events[2].target.is_some());
        assert!(options.apply.is_some());
    }
    #[test]
    fn rejects_unknown_options() {
        let input = syn::parse_quote!(
            #[contime(hahs = false)]
            struct E {
                time: i64,
            }
        );
        assert!(Options::read(&input, false).is_err());
    }
    #[test]
    fn parses_module_time() {
        let input = syn::parse_quote!(
            #[contime_module(time = i64, modules = [left::Child], events = [Added], snapshots = [Counter])]
            struct Root;
        );
        let options = Options::read(&input, true).unwrap();
        assert_eq!(options.modules.len(), 1);
        assert!(options.time_type.is_some());
    }

    #[test]
    #[ignore = "Criterion parser benchmark; run explicitly with --ignored --nocapture"]
    fn benchmark_routing_options() {
        let input: syn::DeriveInput = syn::parse_quote! {
            #[contime_snapshot(id=|owner| u128::from(*owner),events=[Added,Other=>[self.other],Multi=>each(self.targets,|item|[item])],apply={for _ in batch.events{}})]
            struct Counter {owner:u64,time:i64}
        };
        let mut criterion = criterion::Criterion::default();
        criterion.bench_function("macros/parse_routing_options", |bench| {
            bench.iter(|| criterion::black_box(Options::read(&input, false).unwrap()))
        });
        criterion.final_summary();
    }
}

pub(crate) fn fields(input: &DeriveInput) -> syn::Result<Vec<(&syn::Ident, &Type)>> {
    match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(fields) => Ok(fields.named.iter().map(|f| (f.ident.as_ref().unwrap(), &f.ty)).collect()),
            _ => Err(syn::Error::new_spanned(input, "ConTime requires named fields")),
        },
        _ => Err(syn::Error::new_spanned(input, "ConTime requires a struct")),
    }
}
pub(crate) fn time(input: &DeriveInput, options: &Options) -> syn::Result<(Expr, Type)> {
    let expr = options.time.clone().unwrap_or_else(|| syn::parse_quote!(self.time));
    let member = match &expr {
        Expr::Field(f) if literal_self(&f.base) => &f.member,
        _ => return Err(syn::Error::new_spanned(expr, "time must name a timestamp field, for example self.time")),
    };
    let ty = fields(input)?
        .into_iter()
        .find(|(id, _)| member == &Member::Named((*id).clone()))
        .map(|(_, ty)| ty.clone())
        .ok_or_else(|| syn::Error::new_spanned(&expr, "timestamp field not found"))?;
    Ok((expr, ty))
}
pub(crate) fn literal_self(expr: &Expr) -> bool {
    matches!(expr, Expr::Path(path) if path.qself.is_none() && path.path.is_ident("self"))
}
pub(crate) fn bounds(mut generics: syn::Generics, fields: &[(&syn::Ident, &Type)], traits: TokenStream) -> syn::Generics {
    for (_, ty) in fields {
        generics.make_where_clause().predicates.push(syn::parse_quote!(#ty: #traits));
    }
    generics
}

#[cfg(test)]
mod field_tests {
    use super::*;
    #[test]
    fn timestamp_requires_a_literal_self_field() {
        let input = syn::parse_quote!(
            #[contime_event(time=other.time)]
            struct Added {
                time: i64,
            }
        );
        let options = Options::read(&input, false).unwrap();
        assert!(time(&input, &options).unwrap_err().to_string().contains("time must name a timestamp field"));
    }
}

/// ID algorithms are supplied by the application, rather than selected by a derive.
pub(crate) fn id_closure<'a>(input: &DeriveInput, options: &'a Options) -> syn::Result<&'a syn::ExprClosure> {
    match &options.id {
        Some(Expr::Closure(closure)) => Ok(closure),
        _ => Err(syn::Error::new_spanned(input, "id must be a closure returning u128")),
    }
}
