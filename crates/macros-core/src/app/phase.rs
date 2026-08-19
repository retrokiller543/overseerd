use proc_macro2::TokenStream;
use quote::quote;
use syn::Path;

use super::model::{Declared, PhaseInput};

/// Expands a lifecycle phase whose successful value continues host construction.
pub(super) fn call(
    phase: Option<&Declared<PhaseInput>>,
    default: TokenStream,
    values: &[TokenStream],
    lifecycle_phase: TokenStream,
    phase_error: &Path,
) -> TokenStream {
    match phase.map(|phase| &phase.value) {
        Some(PhaseInput::Path(path)) => quote! {
            #path(#(#values),*)
                .await
                .map_err(|source| #phase_error::new(#lifecycle_phase, source))?
        },
        Some(PhaseInput::Inline { arguments, body }) => {
            let arguments = arguments.iter().map(|argument| &argument.ident);

            quote! {
                {
                    let (#(#arguments,)*) = (#(#values,)*);

                    (async move #body)
                        .await
                        .map_err(|source| #phase_error::new(#lifecycle_phase, source))?
                }
            }
        }
        None => default,
    }
}

/// Expands a lifecycle phase whose result is returned directly.
pub(super) fn result(
    phase: Option<&Declared<PhaseInput>>,
    default: TokenStream,
    values: &[TokenStream],
    lifecycle_phase: TokenStream,
    phase_error: &Path,
) -> TokenStream {
    match phase.map(|phase| &phase.value) {
        Some(PhaseInput::Path(path)) => quote! {
            #path(#(#values),*)
                .await
                .map_err(|source| #phase_error::new(#lifecycle_phase, source))
        },
        Some(PhaseInput::Inline { arguments, body }) => {
            let arguments = arguments.iter().map(|argument| &argument.ident);

            quote! {
                {
                    let (#(#arguments,)*) = (#(#values,)*);

                    (async move #body)
                        .await
                        .map_err(|source| #phase_error::new(#lifecycle_phase, source))
                }
            }
        }
        None => quote!(Ok(#default)),
    }
}
