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
use syn::{Fields, Generics, Ident, ItemEnum, LitStr, Path, Token, Type, parse_quote};

use overseerd_macros_core::paths::Paths;

/// One segment of a parsed destination template.
#[derive(Debug, PartialEq, Eq)]
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

/// The `#[topics(...)]` arguments: an optional `protocol = <Path>` (the pub/sub protocol, default
/// `Stomp`) and an optional `codec = <Path>` (the body codec, default the protocol's `DefaultCodec`).
pub struct TopicsArgs {
    protocol: Option<Path>,
    codec: Option<Path>,
}

impl Parse for TopicsArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut protocol = None;
        let mut codec = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;

            match key.to_string().as_str() {
                "protocol" => {
                    input.parse::<Token![=]>()?;
                    protocol = Some(input.parse()?);
                }

                "codec" => {
                    input.parse::<Token![=]>()?;
                    codec = Some(input.parse()?);
                }

                _ => {
                    return Err(syn::Error::new_spanned(
                        &key,
                        "unknown #[topics] argument (expected `protocol = <Type>` or `codec = <Type>`)",
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Self { protocol, codec })
    }
}

/// Expands `#[topics]` on an enum into the clean enum plus its `Topic` impl and typed client.
pub fn expand(args: TopicsArgs, mut item: ItemEnum, paths: &Paths) -> syn::Result<TokenStream> {
    let variants = parse_variants(&mut item)?;
    let enum_ident = &item.ident;
    let generics = item.generics.clone();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let topic_trait = paths.plugin("Topic");
    let topic_param = paths.plugin("TopicParam");
    let topic_protocol = paths.plugin("TopicProtocol");
    let topic_codec = paths.plugin("TopicCodec");
    let codec_error = paths.plugin("CodecError");

    // The pub/sub protocol this topic set is published over: the user's `protocol = ..` or the
    // default `Stomp`. Everything the macro emits is generic over it (`<P as TopicProtocol>::Body`,
    // `TopicCodec<P>`, the client's `TopicSubscribe<P>`), so a new protocol needs no macro change.
    let protocol = match &args.protocol {
        Some(path) => quote!(#path),

        None => {
            let stomp = paths.plugin("Stomp");

            quote!(#stomp)
        }
    };

    // The body codec for this topic set: the user's `codec = ..` or the protocol's `DefaultCodec`.
    // Both `Topic::encode` (server publish) and the client `subscribe_*` decode route through it.
    let codec = match &args.codec {
        Some(path) => quote!(#path),

        None => quote!(<#protocol as #topic_protocol>::DefaultCodec),
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
                let build =
                    render_destination(&variant.segments, &topic_param, |hole| quote!(#hole));

                quote!(#enum_ident::#ident { #(#bindings,)* .. } => #build)
            }
        }
    });

    let encode_arms = variants.iter().map(|variant| {
        let ident = &variant.ident;

        match &variant.kind {
            VariantKind::Static { .. } => {
                quote!(#enum_ident::#ident(__payload) => <#codec as #topic_codec<#protocol>>::encode(__payload))
            }

            VariantKind::Templated { content_field, .. } => {
                quote!(#enum_ident::#ident { #content_field, .. } => <#codec as #topic_codec<#protocol>>::encode(#content_field))
            }
        }
    });

    let topic_impl = quote! {
        impl #impl_generics #topic_trait for #enum_ident #ty_generics #where_clause {
            type Protocol = #protocol;

            fn destination(&self) -> ::std::borrow::Cow<'static, str> {
                match self {
                    #(#destination_arms),*
                }
            }

            fn encode(&self) -> ::core::result::Result<<#protocol as #topic_protocol>::Body, #codec_error> {
                match self {
                    #(#encode_arms),*
                }
            }
        }
    };

    let client = generate_client(enum_ident, &generics, &variants, &protocol, &codec, paths);
    let wasm_client = generate_wasm_client(enum_ident, &generics, &variants, &protocol, paths);

    Ok(quote! {
        #item

        #topic_impl

        #client

        #wasm_client
    })
}

/// Emits `{Enum}Client<C>` with one typed `subscribe_<variant>()` per topic. A templated topic's
/// method takes its params as typed arguments and renders the destination at the call site. Empty
/// without the macro crate's `client` feature (mirroring the HTTP client codegen gate).
fn generate_client(
    enum_ident: &Ident,
    generics: &Generics,
    variants: &[TopicVariant],
    protocol: &TokenStream,
    codec: &TokenStream,
    paths: &Paths,
) -> TokenStream {
    if !cfg!(feature = "client") {
        return quote!();
    }

    let client_ident = format_ident!("{}Client", enum_ident);

    // Weave the topic enum's own generics (`<'a>`, `<T>`, ..) into the client so a generic or
    // borrowing topic set keeps its parameters. The client wraps a transport param plus the enum's
    // params; the enum's `where`-clause (e.g. `T: DeserializeOwned`) rides along so a `subscribe_*`
    // that decodes into a generic payload stays well-formed. A borrowing payload works too when it
    // is `DeserializeOwned` (e.g. `Cow<'a, T>`, which serializes zero-copy yet decodes to `Owned`).
    //
    // The transport param is `C` unless the enum already declares a `C` (type or const) generic —
    // then it would collide, so a `__`-prefixed fallback is used instead. Threaded through every
    // client signature as `#transport` so the two never clash.
    let transport = transport_param_ident(generics);

    // A `subscribe_*` yields a stream of *decoded, owned* payloads, which the subscription stream
    // requires to be `M: 'static`. So a lifetime the payload borrows over must be pinned to
    // `'static` on the client side (a `Cow<'a, T>` still satisfies it — it decodes to `Owned`). But
    // a lifetime used *only* by a templated destination param (e.g. `room: &'a str`) is consumed
    // synchronously while rendering the destination and never reaches the stream, so it stays free —
    // pinning it would reject valid non-`'static` call sites. Bind `'static` to exactly the
    // lifetimes that appear in some variant's payload type.
    let content_lifetimes: std::collections::HashSet<String> = variants
        .iter()
        .flat_map(|variant| match &variant.kind {
            VariantKind::Static { payload } => lifetimes_in_type(payload),
            VariantKind::Templated { content_type, .. } => lifetimes_in_type(content_type),
        })
        .collect();

    let enum_lifetimes: Vec<_> = generics.lifetimes().map(|lt| lt.lifetime.clone()).collect();

    let mut client_generics = generics.clone();
    {
        let where_clause = client_generics.make_where_clause();

        for lifetime in &enum_lifetimes {
            if content_lifetimes.contains(&lifetime.ident.to_string()) {
                where_clause
                    .predicates
                    .push(parse_quote!(#lifetime: 'static));
            }
        }
    }
    client_generics.params.push(parse_quote!(#transport));

    let (client_impl_generics, client_ty_generics, client_where) = client_generics.split_for_impl();
    let (_, enum_ty_generics, _) = generics.split_for_impl();

    // A generic enum's params must appear in the struct body; a `PhantomData` over the enum type
    // carries them (and their variance). A non-generic enum keeps the bare newtype so the common
    // case — and every existing `{Enum}Client::new(..).0` access — is byte-for-byte unchanged.
    let has_generics = !generics.params.is_empty();

    let (struct_fields, phantom_init) = if has_generics {
        (
            quote!((pub #transport, ::core::marker::PhantomData<#enum_ident #enum_ty_generics>)),
            quote!(, ::core::marker::PhantomData),
        )
    } else {
        (quote!((pub #transport)), quote!())
    };
    let subscription = paths.plugin("client::Subscription");
    let topic_subscribe = paths.plugin("client::TopicSubscribe");
    let topic_codec = paths.plugin("TopicCodec");
    let topic_client_protocol = paths.plugin("TopicClientProtocol");
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
            ) -> ::core::result::Result<
                #subscription<#protocol, #transport, #msg>,
                #client_error<<#protocol as #topic_client_protocol>::Status>,
            >
            where
                #transport: #topic_subscribe<#protocol> + ::core::clone::Clone,
            {
                // The topic set's codec decodes each inbound message body into `#msg`.
                <#transport as #topic_subscribe<#protocol>>::subscribe::<#msg>(
                    &self.0,
                    #dest_expr,
                    <#codec as #topic_codec<#protocol>>::decode::<#msg>,
                )
                .await
            }
        }
    });

    quote! {
        #[doc = concat!("Generated STOMP subscription client for the `", stringify!(#enum_ident), "` topics.")]
        pub struct #client_ident #client_impl_generics #struct_fields #client_where;

        impl #client_impl_generics #client_ident #client_ty_generics #client_where {
            /// Wraps a STOMP client transport.
            pub fn new(transport: #transport) -> Self {
                Self(transport #phantom_init)
            }

            #(#methods)*
        }
    }
}

/// Emits the **wasm** JS binding for the topics subscribe client: a `#[wasm_bindgen]` newtype over
/// `{Enum}Client<StompClientTransport>`, built from the shared [`Connection`], with one
/// `subscribe_<variant>()` per topic. Each takes a **typed** callback (`(message: T) => void`, via a
/// per-method `typescript_type` extern so TS sees the real message type) and returns a
/// `StompSubscription` handle. wasm-only; requires the fetch backend (`Connection`) and the ws
/// transport (`StompClientTransport`), so it is gated on both `reqwest` and `tungstenite`.
fn generate_wasm_client(
    enum_ident: &Ident,
    generics: &Generics,
    variants: &[TopicVariant],
    protocol: &TokenStream,
    paths: &Paths,
) -> TokenStream {
    if !(cfg!(feature = "reqwest") && cfg!(feature = "tungstenite")) {
        return quote!();
    }

    // A `#[wasm_bindgen]` type cannot be generic, so a generic/borrowing topic set exposes no JS
    // binding (its native `{Enum}Client<C>` still works). The concrete common case is unaffected.
    if !generics.params.is_empty() {
        return quote!();
    }

    let client_ident = format_ident!("{}Client", enum_ident);
    let js_name = client_ident.to_string();
    let wrapper = format_ident!("__{}Wasm", client_ident);
    let connection = paths.plugin("client::Connection");
    let topic_wasm_client = paths.plugin("client::TopicWasmClient");
    let pump = paths.plugin("client::pump");
    let subscription = paths.plugin("client::TopicSubscription");
    let ts = cfg!(feature = "wasm-ts");

    // Per-method typed callback extern + the subscribe method itself.
    let mut handlers = Vec::new();
    let mut methods = Vec::new();

    for variant in variants {
        let method = format_ident!("subscribe_{}", to_snake_case(&variant.ident.to_string()));
        let handler = format_ident!("__{}{}Handler", enum_ident, variant.ident);

        let msg = match &variant.kind {
            VariantKind::Static { payload } => payload,
            VariantKind::Templated { content_type, .. } => content_type,
        };
        let ts_msg = ts_type_name(msg);
        let handler_ts = format!("(message: {ts_msg}) => void");

        // Templated topics take their hole params (typed for TS); a static topic takes none.
        let params = match &variant.kind {
            VariantKind::Templated { params, .. } => params.clone(),
            VariantKind::Static { .. } => Vec::new(),
        };
        let param_decls = params.iter().map(|(name, ty)| {
            if ts {
                quote!(, #name: ::tsify::Ts<#ty>)
            } else {
                quote!(, #name: #ty)
            }
        });
        let param_convs = params.iter().map(|(name, _)| {
            if ts {
                quote!(let #name = #name.to_rust().map_err(::wasm_bindgen::JsError::from)?;)
            } else {
                quote!()
            }
        });
        let call_args = params.iter().map(|(name, _)| quote!(#name));

        handlers.push(quote! {
            #[cfg(target_family = "wasm")]
            #[::wasm_bindgen::prelude::wasm_bindgen]
            extern "C" {
                #[::wasm_bindgen::prelude::wasm_bindgen(typescript_type = #handler_ts)]
                type #handler;
            }
        });

        methods.push(quote! {
            pub async fn #method(
                &self #(#param_decls)*,
                on_message: #handler,
            ) -> ::core::result::Result<#subscription, ::wasm_bindgen::JsError> {
                #(#param_convs)*

                let __sub = self
                    .0
                    .#method(#(#call_args),*)
                    .await
                    .map_err(|e| ::wasm_bindgen::JsError::new(&::std::string::ToString::to_string(&e)))?;

                ::core::result::Result::Ok(#pump(
                    __sub,
                    ::wasm_bindgen::JsCast::unchecked_into(on_message),
                ))
            }
        });
    }

    quote! {
        #(#handlers)*

        #[cfg(target_family = "wasm")]
        #[doc(hidden)]
        #[::wasm_bindgen::prelude::wasm_bindgen(js_name = #js_name)]
        pub struct #wrapper(#client_ident<<#protocol as #topic_wasm_client>::Transport>);

        #[cfg(target_family = "wasm")]
        #[::wasm_bindgen::prelude::wasm_bindgen(js_class = #js_name)]
        impl #wrapper {
            /// Builds the subscribe client from a shared [`Connection`] (the protocol's socket must be
            /// connected first).
            #[::wasm_bindgen::prelude::wasm_bindgen(constructor)]
            pub fn new(connection: &#connection) -> ::core::result::Result<#wrapper, ::wasm_bindgen::JsError> {
                ::core::result::Result::Ok(Self(#client_ident::new(
                    <#protocol as #topic_wasm_client>::transport(connection)?,
                )))
            }

            #(#methods)*
        }
    }
}

/// The TypeScript type name for a payload type, so a typed callback reads `(message: T) => void`.
/// Maps the common primitives to their TS names; a user `#[dto]` type keeps its ident (which is the
/// name `tsify` generates). A path type falls back to its last segment.
/// Collects the names of every lifetime mentioned in `ty` (`&'a str` → `{"a"}`). Walks the type's
/// token stream (a lifetime is a `'` punct joined to an ident) rather than pulling in syn's `visit`
/// feature. Used to decide which client lifetimes a borrowing payload pins to `'static`.
fn lifetimes_in_type(ty: &Type) -> std::collections::HashSet<String> {
    use proc_macro2::TokenTree;

    fn walk(tokens: TokenStream, out: &mut std::collections::HashSet<String>) {
        let mut iter = tokens.into_iter().peekable();

        while let Some(tree) = iter.next() {
            match tree {
                TokenTree::Punct(punct) if punct.as_char() == '\'' => {
                    if let Some(TokenTree::Ident(ident)) = iter.peek() {
                        out.insert(ident.to_string());
                        iter.next();
                    }
                }

                TokenTree::Group(group) => walk(group.stream(), out),

                _ => {}
            }
        }
    }

    let mut out = std::collections::HashSet::new();

    walk(quote!(#ty), &mut out);

    out
}

fn ts_type_name(ty: &Type) -> String {
    let ident = match ty {
        Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    };

    match ident.as_deref() {
        Some("String" | "str" | "char") => "string".to_owned(),
        Some("bool") => "boolean".to_owned(),
        Some(
            "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
            | "isize" | "f32" | "f64",
        ) => "number".to_owned(),
        Some(name) => name.to_owned(),
        None => "any".to_owned(),
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
        let position = variant
            .attrs
            .iter()
            .position(is_topic_attr)
            .ok_or_else(|| {
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

            Fields::Named(fields) => {
                parse_templated_variant(&variant.ident, &destination, &holes, fields)?
            }

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
        let field = field_types
            .iter()
            .find(|(name, _)| &name.to_string() == hole);

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
                let mut closed = false;

                for inner in chars.by_ref() {
                    if inner == '}' {
                        closed = true;
                        break;
                    }

                    name.push(inner);
                }

                if !closed {
                    return Err(syn::Error::new_spanned(
                        destination,
                        "unmatched `{` in topic template (missing closing `}`)",
                    ));
                }

                if name.is_empty() {
                    return Err(syn::Error::new_spanned(
                        destination,
                        "empty `{}` hole in topic template",
                    ));
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

/// The client's transport type-parameter ident: `C`, unless the topic enum already declares a `C`
/// type or const generic — appending our own `C` would then be a duplicate-parameter error. In that
/// (rare) case a `__`-prefixed fallback is used, and if *that* also collides a numeric suffix is
/// appended until the ident is guaranteed fresh against the enum's full generic parameter set.
fn transport_param_ident(generics: &Generics) -> Ident {
    let declared: std::collections::HashSet<String> = generics
        .type_params()
        .map(|param| param.ident.to_string())
        .chain(generics.const_params().map(|param| param.ident.to_string()))
        .collect();

    if !declared.contains("C") {
        return format_ident!("C");
    }

    let base = "__OverseerdClientTransport";

    if !declared.contains(base) {
        return format_ident!("{base}");
    }

    let mut suffix = 0usize;

    loop {
        let candidate = format!("{base}{suffix}");

        if !declared.contains(&candidate) {
            return format_ident!("{candidate}");
        }

        suffix += 1;
    }
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

#[cfg(test)]
mod tests;
