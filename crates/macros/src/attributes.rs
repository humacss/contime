//! Module registrations and optional explicit metadata for handwritten snapshots.
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
    pub infer_events: bool,
}
#[derive(Clone)]
pub struct Lane {
    pub name: Ident,
    pub path: Path,
    pub subset: Option<Vec<Lane>>,
}
impl Parse for Lane {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let (name, path) = if input.peek(Ident) && input.peek2(Token![=]) {
            let name = input.parse()?;
            input.parse::<Token![=]>()?;
            (name, input.parse()?)
        } else {
            let path: Path = input.parse()?;
            (path.segments.last().unwrap().ident.clone(), path)
        };
        Ok(Self { name, path, subset: None })
    }
}
fn lanes(input: ParseStream<'_>) -> syn::Result<Vec<Lane>> {
    let list;
    syn::bracketed!(list in input);
    Ok(Punctuated::<Lane, Token![,]>::parse_terminated(&list)?.into_iter().collect())
}
fn paths(input: ParseStream<'_>) -> syn::Result<Vec<Path>> {
    let list;
    syn::bracketed!(list in input);
    Ok(Punctuated::<Path, Token![,]>::parse_terminated(&list)?.into_iter().collect())
}
impl Options {
    pub fn read(input: &DeriveInput) -> syn::Result<Self> {
        let mut out = Self::default();
        let mut seen = std::collections::HashSet::new();
        let mut modern = false;
        let mut explicit_events = false;
        let mut legacy = false;
        for attr in &input.attrs {
            if attr.path().is_ident("contime_module") {
                legacy = true;
                attr.parse_nested_meta(|meta| {
                    let name = meta.path.get_ident().ok_or_else(|| meta.error("expected an option name"))?.to_string();
                    if !seen.insert(name.clone()) {
                        return Err(meta.error("duplicate module option"));
                    }
                    let value = meta.value()?;
                    match name.as_str() {
                        "time" => out.time_type = Some(value.parse()?),
                        "events" => {
                            out.events = lanes(value)?;
                            explicit_events = true;
                        }
                        "snapshots" => out.snapshots = lanes(value)?,
                        "modules" => out.modules = paths(value)?,
                        _ => return Err(meta.error("expected time, events, snapshots or modules")),
                    }
                    Ok(())
                })?;
            } else if attr.path().is_ident("contime") {
                modern = true;
                let mut kind = None;
                let mut snapshot = None;
                attr.parse_nested_meta(|meta| {
                    let key = meta.path.get_ident().ok_or_else(|| meta.error("expected a registration name"))?.to_string();
                    if kind.is_none() {
                        kind = Some(key.clone());
                        match key.as_str() {
                            "snapshot" => snapshot = Some(meta.value()?.parse::<Lane>()?),
                            "snapshots" => out.snapshots.extend(lanes(meta.value()?)?),
                            "modules" => out.modules.extend(paths(meta.value()?)?),
                            "events" => {
                                if explicit_events {
                                    return Err(meta.error("duplicate explicit event list"));
                                }
                                out.events = lanes(meta.value()?)?;
                                explicit_events = true;
                            }
                            "time" => {
                                if out.time_type.is_some() {
                                    return Err(meta.error("duplicate module time"));
                                }
                                out.time_type = Some(meta.value()?.parse()?);
                            }
                            _ => {
                                return Err(meta.error(
                                    "expected snapshot, snapshots or modules; explicit events/time are available for handwritten metadata",
                                ))
                            }
                        }
                    } else if kind.as_deref() == Some("snapshot") && key == "events" {
                        let snapshot = snapshot.as_mut().unwrap();
                        if snapshot.subset.is_some() {
                            return Err(meta.error("duplicate snapshot event subset"));
                        }
                        snapshot.subset = Some(lanes(meta.value()?)?);
                    } else {
                        return Err(meta.error("customize one snapshot with snapshot = Type, events = [...]"));
                    }
                    Ok(())
                })?;
                if let Some(snapshot) = snapshot {
                    out.snapshots.push(snapshot);
                }
            }
        }
        if modern && legacy {
            return Err(syn::Error::new_spanned(input, "use contime or contime_module consistently on a module"));
        }
        out.infer_events = modern && !explicit_events;
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
