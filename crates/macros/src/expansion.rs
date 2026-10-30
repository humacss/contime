//! Resolve the consumer crate and turn expansion errors into compiler diagnostics.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};

pub(crate) fn root() -> Tokens {
    match proc_macro_crate::crate_name("contime") {
        Ok(proc_macro_crate::FoundCrate::Name(name)) => {
            let name = format_ident!("{}", name);
            quote!(::#name)
        }
        _ => quote!(::contime),
    }
}
pub(crate) fn result(result: syn::Result<Tokens>) -> TokenStream {
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}
