//! The `#[topics]` macro: turns a user enum into the shared client+server topic contract.
//!
//! Each variant is one broadcast topic — a `#[topic("/topic/..")]` destination carrying a single
//! payload type. The macro emits an `impl Topic` (destination + JSON encode) so the server can
//! publish typed values, and (behind the `client` feature) a `{Enum}Client<C>` with one
//! `subscribe_<variant>()` method per topic returning a typed `Subscription`. The enum is the
//! single source of truth: a renamed destination or changed payload type is a compile error on
//! whichever side lags.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Fields, ItemEnum, LitStr, Path, Token, Type};

use overseerd_macros_core::paths::Paths;

/// One parsed topic variant: its name, destination, and payload type.
struct TopicVariant {
    ident: syn::Ident,
    destination: LitStr,
    payload: Type,
}

/// The `#[topics(...)]` arguments — currently just an optional `codec = <Path>`.
pub struct TopicsArgs {
    codec: Option<Path>,
}

impl Parse for TopicsArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut codec = None;

        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;

            match key.to_string().as_str() {
                "codec" => {
                    input.parse::<Token![=]>()?;
                    codec = Some(input.parse()?);
                }

                _ => {
                    return Err(syn::Error::new_spanned(
                        &key,
                        "unknown #[topics] argument (expected `codec = <Type>`)",
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Self { codec })
    }
}

/// Expands `#[topics]` on an enum into the clean enum plus its `Topic` impl and typed client.
pub fn expand(args: TopicsArgs, mut item: ItemEnum, paths: &Paths) -> syn::Result<TokenStream> {
    let variants = parse_variants(&mut item)?;
    let enum_ident = &item.ident;

    let topic_trait = paths.plugin("Topic");
    let stomp_codec = paths.plugin("StompCodec");
    let stomp_body = paths.plugin("StompBody");
    let codec_error = paths.plugin("CodecError");

    // The body codec for this topic set: the user's `codec = ..` or the default `JsonCodec`. Both
    // `Topic::encode` (server publish) and the client `subscribe_*` decode route through it.
    let codec = match args.codec {
        Some(path) => quote!(#path),

        None => {
            let json_codec = paths.plugin("JsonCodec");

            quote!(#json_codec)
        }
    };

    let destination_arms = variants.iter().map(|variant| {
        let ident = &variant.ident;
        let destination = &variant.destination;

        quote!(#enum_ident::#ident(_) => #destination)
    });

    let encode_arms = variants.iter().map(|variant| {
        let ident = &variant.ident;

        quote!(#enum_ident::#ident(__payload) => <#codec as #stomp_codec>::encode(__payload))
    });

    let topic_impl = quote! {
        impl #topic_trait for #enum_ident {
            fn destination(&self) -> &'static str {
                match self {
                    #(#destination_arms),*
                }
            }

            fn encode(&self) -> ::core::result::Result<#stomp_body, #codec_error> {
                match self {
                    #(#encode_arms),*
                }
            }
        }
    };

    let client = generate_client(enum_ident, &variants, &codec, paths);

    Ok(quote! {
        #item

        #topic_impl

        #client
    })
}

/// Emits `{Enum}Client<C>` with one typed `subscribe_<variant>()` per topic. Empty without the
/// macro crate's `client` feature (mirroring the HTTP client codegen gate).
fn generate_client(
    enum_ident: &syn::Ident,
    variants: &[TopicVariant],
    codec: &TokenStream,
    paths: &Paths,
) -> TokenStream {
    if !cfg!(feature = "client") {
        return quote!();
    }

    let client_ident = format_ident!("{}Client", enum_ident);
    let subscription = paths.plugin("Subscription");
    let stomp_subscribe = paths.plugin("StompSubscribe");
    let stomp_codec = paths.plugin("StompCodec");
    let stomp_status = paths.plugin("StompStatus");
    let client_error = paths.client("ClientError");

    let methods = variants.iter().map(|variant| {
        let method = format_ident!("subscribe_{}", to_snake_case(&variant.ident.to_string()));
        let msg = &variant.payload;
        let destination = &variant.destination;

        quote! {
            #[doc = concat!("Subscribes to `", #destination, "`, yielding a typed stream of messages.")]
            pub async fn #method(
                &self,
            ) -> ::core::result::Result<#subscription<C, #msg>, #client_error<#stomp_status>>
            where
                C: #stomp_subscribe + ::core::clone::Clone,
            {
                // The topic set's codec decodes each MESSAGE body into `#msg`.
                <C as #stomp_subscribe>::stomp_subscribe::<#msg>(
                    &self.0,
                    #destination,
                    <#codec as #stomp_codec>::decode::<#msg>,
                )
                .await
            }
        }
    });

    quote! {
        #[doc = concat!("Generated STOMP subscription client for the `", stringify!(#enum_ident), "` topics.")]
        pub struct #client_ident<C>(pub C);

        impl<C> #client_ident<C> {
            /// Wraps a STOMP client transport.
            pub fn new(transport: C) -> Self {
                Self(transport)
            }

            #(#methods)*
        }
    }
}

/// Parses (and strips) the `#[topic("..")]` attribute and single payload type off each variant.
fn parse_variants(item: &mut ItemEnum) -> syn::Result<Vec<TopicVariant>> {
    let mut variants = Vec::new();

    for variant in &mut item.variants {
        let position = variant.attrs.iter().position(is_topic_attr).ok_or_else(|| {
            syn::Error::new_spanned(
                &variant.ident,
                "every #[topics] variant needs a #[topic(\"/topic/..\")] attribute",
            )
        })?;

        let attr = variant.attrs.remove(position);
        let destination: LitStr = attr.parse_args()?;

        let payload = match &variant.fields {
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                fields.unnamed[0].ty.clone()
            }

            _ => {
                return Err(syn::Error::new_spanned(
                    &variant.ident,
                    "a #[topics] variant must be a tuple variant with exactly one payload type, \
                     e.g. `Room(RoomMsg)`",
                ));
            }
        };

        variants.push(TopicVariant {
            ident: variant.ident.clone(),
            destination,
            payload,
        });
    }

    if variants.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "a #[topics] enum must declare at least one #[topic(..)] variant",
        ));
    }

    Ok(variants)
}

/// Whether an attribute is `#[topic(..)]`.
fn is_topic_attr(attr: &syn::Attribute) -> bool {
    attr.path().is_ident("topic")
}

/// Lower-snake-cases a variant ident (`RoomUpdates` → `room_updates`).
fn to_snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);

    for (index, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if index != 0 {
                out.push('_');
            }

            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }

    out
}
