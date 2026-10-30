//! Nominal identities used only while composing generated Rust lanes.
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_keys_are_opaque_and_distinguish_repeated_definitions() {
        let name = syn::parse_quote!(App);
        let first = key(&name);
        let second = key(&name);
        assert_ne!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
