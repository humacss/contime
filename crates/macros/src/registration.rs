//! Nominal identities used only while composing generated Rust lanes.
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::sync::atomic::{AtomicUsize, Ordering};
use syn::Ident;

// Source spans can refer to one macro definition for several distinct expansions.
// This serial distinguishes those definitions within the compiler process. The
// resulting keys are composition metadata, never event or snapshot identities.
static NEXT_DEFINITION: AtomicUsize = AtomicUsize::new(0);

pub fn key(name: &Ident) -> String {
    let span = name.span();
    let definition = format!(
        "{}:{}:{}:{}:{}:{}:{}:{}",
        std::env::var("CARGO_PKG_NAME").unwrap_or_default(),
        std::env::var("CARGO_PKG_VERSION").unwrap_or_default(),
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default(),
        span.file(),
        span.start().line,
        span.start().column,
        name,
        NEXT_DEFINITION.fetch_add(1, Ordering::Relaxed)
    );
    // Only opaque fingerprints enter exported composition metadata.
    blake3::hash(definition.as_bytes()).to_hex().to_string()
}
pub fn suffix(key: &str) -> String {
    format!("{:016x}", key.bytes().fold(0xcbf29ce484222325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)))
}
pub fn metadata(name: &Ident) -> TokenStream {
    let key = key(name);
    let exported = format_ident!("__contime_type_{}_{}", name, suffix(&key));
    quote! {
        #[doc(hidden)] #[macro_export] macro_rules! #exported {
            (@contime_definition $callback:path; $($state:tt)*)=>{$callback!{#key $($state)*}};
        }
        #[doc(hidden)] #[allow(unused_imports)] pub use #exported as #name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nominal_keys_distinguish_definitions_and_metadata_forwards_callbacks() {
        let first = syn::parse_quote!(First);
        let second = syn::parse_quote!(Second);
        assert_ne!(key(&first), key(&second));
        assert_ne!(suffix(&key(&first)), suffix(&key(&second)));
        let generated = metadata(&first).to_string();
        assert!(generated.contains("$ callback !"));
        assert!(generated.contains("as First"));
    }
    #[test]
    fn registration_keys_are_opaque() {
        let name = syn::parse_quote!(Added);
        let key = key(&name);
        assert_eq!(key.len(), 64, "registration keys must be digests, not source metadata");
        assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    #[test]
    fn exported_metadata_contains_only_an_opaque_key() {
        fn literals(tokens: TokenStream, values: &mut Vec<String>) {
            for token in tokens {
                match token {
                    proc_macro2::TokenTree::Group(group) => literals(group.stream(), values),
                    proc_macro2::TokenTree::Literal(literal) => {
                        let literal: syn::LitStr = syn::parse_str(&literal.to_string()).unwrap();
                        values.push(literal.value());
                    }
                    _ => {}
                }
            }
        }
        let name = syn::parse_quote!(Added);
        let mut values = Vec::new();
        literals(metadata(&name), &mut values);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].len(), 64, "exported metadata must not contain raw definition keys");
        assert!(values[0].bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    #[test]
    fn repeated_expansions_with_identical_spans_have_distinct_keys() {
        let name = syn::parse_quote!(Added);
        assert_ne!(key(&name), key(&name));
    }
    #[test]
    #[ignore = "Criterion registration benchmark; run explicitly with --ignored --nocapture"]
    fn benchmark_definition_metadata() {
        let name = syn::parse_quote!(Counter);
        let mut criterion = criterion::Criterion::default();
        criterion.bench_function("macros/definition_metadata", |bench| bench.iter(|| criterion::black_box(metadata(&name))));
        criterion.final_summary();
    }
}
