//! Explicit registration names are checked against actual Rust type equality.
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    DeriveInput, Ident, Path, Token, Type,
};
#[derive(Default)]
pub struct Options {
    pub time_type: Option<Type>,
    pub events: Vec<Lane>,
    pub snapshots: Vec<Lane>,
    pub modules: Vec<Path>,
}
pub struct Lane {
    pub name: Ident,
    pub path: Path,
}
impl Parse for Lane {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Ident) && input.peek2(Token![=]) {
            let name = input.parse()?;
            input.parse::<Token![=]>()?;
            Ok(Self { name, path: input.parse()? })
        } else {
            let path: Path = input.parse()?;
            Ok(Self { name: path.segments.last().unwrap().ident.clone(), path })
        }
    }
}
impl Options {
    pub fn read(input: &DeriveInput) -> syn::Result<Self> {
        let mut out = Self::default();
        let mut seen = std::collections::HashSet::new();
        for attr in &input.attrs {
            if !attr.path().is_ident("contime_module") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                let name = meta.path.get_ident().ok_or_else(|| meta.error("expected an option name"))?.to_string();
                if !seen.insert(name.clone()) {
                    return Err(meta.error("duplicate module option"));
                }
                let value = meta.value()?;
                match name.as_str() {
                    "time" => out.time_type = Some(value.parse()?),
                    "events" | "snapshots" => {
                        let list;
                        syn::bracketed!(list in value);
                        let lanes = Punctuated::<Lane, Token![,]>::parse_terminated(&list)?.into_iter().collect();
                        if name == "events" {
                            out.events = lanes;
                        } else {
                            out.snapshots = lanes;
                        }
                    }
                    "modules" => {
                        let list;
                        syn::bracketed!(list in value);
                        out.modules = Punctuated::<Path, Token![,]>::parse_terminated(&list)?.into_iter().collect();
                    }
                    _ => return Err(meta.error("expected time, events, snapshots or modules")),
                }
                Ok(())
            })?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_names_and_bare_types_share_the_same_list_syntax() {
        let input = syn::parse_quote!(
            #[contime_module(time=i64,events=[Added,Added=Incoming],snapshots=[State=domain::Total],modules=[child::Module])]
            struct App;
        );
        let options = Options::read(&input).unwrap();
        assert_eq!(options.events[0].name, options.events[1].name);
        assert!(options.events[1].path.is_ident("Incoming"));
        assert_eq!(options.snapshots[0].name, "State");
        assert_eq!(options.modules.len(), 1);
    }
    #[test]
    fn repeated_options_and_nonmodule_options_are_rejected() {
        for input in [
            syn::parse_quote!(
                #[contime_module(time=i64,time=u64)]
                struct App;
            ),
            syn::parse_quote!(
                #[contime_module(hash = true)]
                struct App;
            ),
        ] {
            assert!(Options::read(&input).is_err());
        }
    }
    #[test]
    #[ignore = "Criterion parser benchmark; run explicitly with --ignored --nocapture"]
    fn benchmark_module_options() {
        let input = syn::parse_quote!(
            #[contime_module(time=i64,events=[Added,Other],snapshots=[Counter],modules=[Shared])]
            struct App;
        );
        let mut criterion = criterion::Criterion::default();
        criterion.bench_function("macros/parse_module_options", |b| b.iter(|| criterion::black_box(Options::read(&input).unwrap())));
        criterion.final_summary();
    }
}
