//! The `#[topics]` macro: turns a user enum into the shared client+server topic contract.
//!
//! Each variant is one broadcast topic. Two shapes:
//!
//! - **Static** — a tuple variant `Chat(ChatMessage)` with a literal `#[topic("/topic/chat")]`.
//! - **Templated** — a struct variant whose named fields fill `{hole}`s in the destination, with a
//!   `#[content]` field for the payload:
//!   `#[topic("/topic/{org}/room/{room}")] Room { org: OrgId, room: String, #[content] msg: RoomMsg }`.
//!   Each hole is a typed field (any [`TopicParam`]); the destination is rendered from them.
//!
//! The macro emits an `impl Topic` (destination + codec encode) for the server publish, and (behind
//! the `client` feature) a `{Enum}Client<C>` with one `subscribe_<variant>()` per topic — taking
//! the template params as typed arguments — returning a typed `Subscription`. The enum is the single
//! source of truth: a renamed destination, changed payload, or changed param type is a compile error
//! on whichever side lags.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Fields, Ident, ItemEnum, LitStr, Path, Token, Type};

use overseerd_macros_core::paths::Paths;

/// One segment of a parsed destination template.
enum Segment {
    /// A literal run of the destination string.
    Literal(String),

    /// A `{name}` hole, filled by the field of the same name.
    Hole(String),
}

/// The shape of one topic variant.
enum VariantKind {
    /// A tuple variant `Chat(Msg)`: literal destination, payload is the single field.
    Static { payload: Type },

    /// A struct variant `Room { org, .., #[content] msg }`: templated destination. `params` are the
    /// hole fields in template order; `content` is the payload field.
    Templated {
        params: Vec<(Ident, Type)>,
        content_field: Ident,
        content_type: Type,
    },
}

/// One parsed topic variant: its name, destination template, and shape.
struct TopicVariant {
    ident: Ident,
    destination: LitStr,
    segments: Vec<Segment>,
    kind: VariantKind,
}

/// The `#[topics(...)]` arguments — currently just an optional `codec = <Path>`.
pub struct TopicsArgs {
    codec: Option<Path>,
}

impl Parse for TopicsArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut codec = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;

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
    let topic_param = paths.plugin("TopicParam");
    let stomp_codec = paths.plugin("StompCodec");
    let stomp_body = paths.plugin("StompBody");
    let codec_error = paths.plugin("CodecError");

    // The body codec for this topic set: the user's `codec = ..` or the default `JsonCodec`. Both
    // `Topic::encode` (server publish) and the client `subscribe_*` decode route through it.
    let codec = match &args.codec {
        Some(path) => quote!(#path),

        None => {
            let json_codec = paths.plugin("JsonCodec");

            quote!(#json_codec)
        }
    };

    let destination_arms = variants.iter().map(|variant| {
        let ident = &variant.ident;

        match &variant.kind {
            VariantKind::Static { .. } => {
                let destination = &variant.destination;

                quote!(#enum_ident::#ident(_) => ::std::borrow::Cow::Borrowed(#destination))
            }

            VariantKind::Templated { params, .. } => {
                let bindings = params.iter().map(|(name, _)| name);
                // The hole fields are bound by reference in the match; `render` takes `&self`, so a
                // by-ref binding calls it directly.
                let build = render_destination(&variant.segments, &topic_param, |hole| quote!(#hole));

                quote!(#enum_ident::#ident { #(#bindings,)* .. } => #build)
            }
        }
    });

    let encode_arms = variants.iter().map(|variant| {
        let ident = &variant.ident;

        match &variant.kind {
            VariantKind::Static { .. } => {
                quote!(#enum_ident::#ident(__payload) => <#codec as #stomp_codec>::encode(__payload))
            }

            VariantKind::Templated { content_field, .. } => {
                quote!(#enum_ident::#ident { #content_field, .. } => <#codec as #stomp_codec>::encode(#content_field))
            }
        }
    });

    let topic_impl = quote! {
        impl #topic_trait for #enum_ident {
            fn destination(&self) -> ::std::borrow::Cow<'static, str> {
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

/// Emits `{Enum}Client<C>` with one typed `subscribe_<variant>()` per topic. A templated topic's
/// method takes its params as typed arguments and renders the destination at the call site. Empty
/// without the macro crate's `client` feature (mirroring the HTTP client codegen gate).
fn generate_client(
    enum_ident: &Ident,
    variants: &[TopicVariant],
    codec: &TokenStream,
    paths: &Paths,
) -> TokenStream {
    if !cfg!(feature = "client") {
        return quote!();
    }

    let client_ident = format_ident!("{}Client", enum_ident);
    let subscription = paths.plugin("client::Subscription");
    let stomp_subscribe = paths.plugin("client::StompSubscribe");
    let stomp_codec = paths.plugin("StompCodec");
    let stomp_status = paths.plugin("client::StompStatus");
    let topic_param = paths.plugin("TopicParam");
    let client_error = paths.client("ClientError");

    let methods = variants.iter().map(|variant| {
        let method = format_ident!("subscribe_{}", to_snake_case(&variant.ident.to_string()));
        let destination = &variant.destination;

        let (msg, args, dest_expr) = match &variant.kind {
            VariantKind::Static { payload } => {
                (payload, quote!(), quote!(#destination))
            }

            VariantKind::Templated { params, content_type, .. } => {
                let args = params.iter().map(|(name, ty)| quote!(, #name: #ty));
                // Client args are owned, so render by reference.
                let build = render_destination(&variant.segments, &topic_param, |hole| quote!(&#hole));

                (content_type, quote!(#(#args)*), quote!(&#build))
            }
        };

        quote! {
            #[doc = concat!("Subscribes to `", #destination, "`, yielding a typed stream of messages.")]
            pub async fn #method(
                &self #args,
            ) -> ::core::result::Result<#subscription<C, #msg>, #client_error<#stomp_status>>
            where
                C: #stomp_subscribe + ::core::clone::Clone,
            {
                // The topic set's codec decodes each MESSAGE body into `#msg`.
                <C as #stomp_subscribe>::stomp_subscribe::<#msg>(
                    &self.0,
                    #dest_expr,
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

/// Builds an expression of type `Cow<'static, str>` that renders `segments` into a destination.
/// `hole_value` maps a hole name to the token expression for its param value (a bare ident on the
/// server, a reference on the client). A single-literal template borrows; anything with a hole
/// builds an owned string.
fn render_destination(
    segments: &[Segment],
    topic_param: &Path,
    hole_value: impl Fn(&Ident) -> TokenStream,
) -> TokenStream {
    let pushes = segments.iter().map(|segment| match segment {
        Segment::Literal(text) => quote!(__dest.push_str(#text);),

        Segment::Hole(name) => {
            let ident = format_ident!("{name}");
            let value = hole_value(&ident);

            quote!(__dest.push_str(&#topic_param::render(#value));)
        }
    });

    quote! {{
        let mut __dest = ::std::string::String::new();
        #(#pushes)*

        ::std::borrow::Cow::Owned(__dest)
    }}
}

/// Parses (and strips) the `#[topic("..")]` attribute and each variant's shape.
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
        let segments = parse_template(&destination)?;
        let holes = hole_names(&segments);

        let kind = match &mut variant.fields {
            // A tuple variant with one field is a static topic. Holes are meaningless without named
            // fields to fill them.
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                if !holes.is_empty() {
                    return Err(syn::Error::new_spanned(
                        &destination,
                        "a templated destination needs a struct variant with a named field per \
                         `{hole}` (and a `#[content]` field), e.g. \
                         `Room { room: String, #[content] msg: Msg }`",
                    ));
                }

                VariantKind::Static {
                    payload: fields.unnamed[0].ty.clone(),
                }
            }

            Fields::Named(fields) => parse_templated_variant(&variant.ident, &destination, &holes, fields)?,

            _ => {
                return Err(syn::Error::new_spanned(
                    &variant.ident,
                    "a #[topics] variant is either a tuple variant with one payload field \
                     (static topic) or a struct variant with a `#[content]` field (templated topic)",
                ));
            }
        };

        variants.push(TopicVariant {
            ident: variant.ident.clone(),
            destination,
            segments,
            kind,
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

/// Validates a struct variant against its template: one `#[content]` field for the payload, and one
/// named field per hole (in template order). Strips the `#[content]` marker.
fn parse_templated_variant(
    variant_ident: &Ident,
    destination: &LitStr,
    holes: &[String],
    fields: &mut syn::FieldsNamed,
) -> syn::Result<VariantKind> {
    let mut content: Option<(Ident, Type)> = None;
    let mut field_types: Vec<(Ident, Type)> = Vec::new();

    for field in &mut fields.named {
        let ident = field.ident.clone().expect("named field has an ident");

        if let Some(position) = field.attrs.iter().position(is_content_attr) {
            field.attrs.remove(position);

            if content.replace((ident.clone(), field.ty.clone())).is_some() {
                return Err(syn::Error::new_spanned(
                    &field.ident,
                    "a templated topic has exactly one #[content] field",
                ));
            }
        } else {
            field_types.push((ident, field.ty.clone()));
        }
    }

    let Some((content_field, content_type)) = content else {
        return Err(syn::Error::new_spanned(
            variant_ident,
            "a struct-variant topic needs a `#[content]` field marking its payload",
        ));
    };

    // Every non-content field must fill a hole, and every hole must have a field — otherwise the
    // template and the variant disagree.
    for (name, _) in &field_types {
        if !holes.iter().any(|hole| hole == &name.to_string()) {
            return Err(syn::Error::new_spanned(
                name,
                format!(
                    "field `{name}` is not used in the topic template `{}`; every non-content \
                     field must fill a `{{{name}}}` hole",
                    destination.value()
                ),
            ));
        }
    }

    // Order the params by hole appearance, so the generated method's argument order matches the
    // destination reading order.
    let mut params = Vec::with_capacity(holes.len());

    for hole in holes {
        let field = field_types.iter().find(|(name, _)| &name.to_string() == hole);

        match field {
            Some((name, ty)) => params.push((name.clone(), ty.clone())),

            None => {
                return Err(syn::Error::new_spanned(
                    destination,
                    format!("template hole `{{{hole}}}` has no matching field on the variant"),
                ));
            }
        }
    }

    Ok(VariantKind::Templated {
        params,
        content_field,
        content_type,
    })
}

/// Parses a destination template into literal and `{hole}` segments. `{{`/`}}` are literal braces.
fn parse_template(destination: &LitStr) -> syn::Result<Vec<Segment>> {
    let text = destination.value();
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }

            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }

            '{' => {
                if !literal.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut literal)));
                }

                let mut name = String::new();

                for inner in chars.by_ref() {
                    if inner == '}' {
                        break;
                    }

                    name.push(inner);
                }

                if name.is_empty() {
                    return Err(syn::Error::new_spanned(destination, "empty `{}` hole in topic template"));
                }

                segments.push(Segment::Hole(name));
            }

            '}' => {
                return Err(syn::Error::new_spanned(
                    destination,
                    "unmatched `}` in topic template (write `}}` for a literal brace)",
                ));
            }

            _ => literal.push(ch),
        }
    }

    if !literal.is_empty() {
        segments.push(Segment::Literal(literal));
    }

    Ok(segments)
}

/// The hole names of a parsed template, in order.
fn hole_names(segments: &[Segment]) -> Vec<String> {
    segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Hole(name) => Some(name.clone()),

            Segment::Literal(_) => None,
        })
        .collect()
}

/// Whether an attribute is `#[topic(..)]`.
fn is_topic_attr(attr: &syn::Attribute) -> bool {
    attr.path().is_ident("topic")
}

/// Whether an attribute is the `#[content]` field marker.
fn is_content_attr(attr: &syn::Attribute) -> bool {
    attr.path().is_ident("content")
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
