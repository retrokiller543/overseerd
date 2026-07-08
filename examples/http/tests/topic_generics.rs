//! Regression: `#[topics]` (and `#[dto]`) must accept lifetimes and generics and generate correct
//! code for them — the macros previously dropped the enum's generics, so a generic or borrowing
//! topic set failed to compile. Covers three shapes:
//!
//!   - a **borrowing** topic set (`Cow<'a, T>`): publishes zero-copy from a borrow, yet its client
//!     decodes into an owned `Cow`, since `Cow<'a, T>` is `DeserializeOwned`;
//!   - a **generic** topic set (`<T>`): payload type chosen by the caller;
//!   - a topic set that itself names a `C` generic, which must not collide with the client's own
//!     transport type parameter.
#![cfg(not(target_family = "wasm"))]

use std::borrow::Cow;

use overseerd::axum::client::StompClientTransport;
use overseerd::axum::*;

#[dto]
#[derive(Clone, PartialEq, Debug)]
pub struct Payload {
    pub text: String,
}

/// A borrowing topic set: the lifetime lets a publisher hand the broker a borrowed payload without
/// cloning, while the subscribe client still decodes an owned value.
#[topics]
pub enum Borrowed<'a> {
    #[topic("/topic/borrowed")]
    Msg(Cow<'a, Payload>),
}

/// A generic topic set: any owned, serializable payload. The subscribe client streams the payload
/// across tasks, so it must be `Send + 'static` — a bound the macro forwards from this `where`.
#[topics]
pub enum Generic<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
{
    #[topic("/topic/generic")]
    Event(T),
}

/// The enum's own `C` generic must not clash with the generated client's transport parameter.
#[topics]
pub enum UsesC<C>
where
    C: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
{
    #[topic("/topic/uses-c")]
    Value(C),
}

/// The `Topic` impl is generated for each shape (borrowed, generic, `C`-named), and a borrowed
/// payload encodes from the borrow.
#[test]
fn topic_impls_are_generated_for_generic_and_borrowing_sets() {
    let payload = Payload {
        text: "hi".to_owned(),
    };

    // Publish borrowed: no clone of `payload` into the topic value.
    let borrowed = Borrowed::Msg(Cow::Borrowed(&payload));
    assert_eq!(borrowed.destination(), "/topic/borrowed");
    assert!(borrowed.encode().is_ok());

    let generic = Generic::Event(payload.clone());
    assert_eq!(generic.destination(), "/topic/generic");
    assert!(generic.encode().is_ok());

    let uses_c = UsesC::Value(payload);
    assert_eq!(uses_c.destination(), "/topic/uses-c");
    assert!(uses_c.encode().is_ok());
}

/// The generated subscribe clients carry the enum's generics too, so they name and construct.
/// (Compile-coverage: a borrowing client decodes into an owned `Cow`, a generic one into `T`.)
#[allow(dead_code)]
fn clients_name_and_construct(transport: StompClientTransport) {
    let _borrowed: BorrowedClient<'static, StompClientTransport> =
        BorrowedClient::new(transport.clone());
    let _generic: GenericClient<Payload, StompClientTransport> =
        GenericClient::new(transport.clone());
    let _uses_c: UsesCClient<Payload, StompClientTransport> = UsesCClient::new(transport);
}
