//! The [`Dto`] marker for data that crosses the HTTP wire.
//!
//! Every value a handler sends or receives on the wire — a JSON/Form request body, a response
//! body, and each path/query parameter — must be a `Dto`. The `#[handlers]` macro asserts this on
//! exactly those positions (never on dependency-injected parameters), so a type that forgot the
//! contract fails with a clear "the type `X` is not a `#[dto]`" error instead of a wall of
//! `IntoResponse`/`Serialize` trait errors.
//!
//! A user type becomes a `Dto` with the [`#[dto]`](macro@crate::dto) attribute, which also derives
//! `serde` (unless `#[dto(no_serde)]`) and — on wasm — `tsify::Tsify`, so the generated browser
//! client is fully typed in TypeScript. The scalar path/query types (`String`, integers, `bool`, …)
//! and the common container shapes are `Dto` out of the box, below.

/// Marks a type as valid HTTP wire data (request/response body, or a path/query parameter). Apply
/// it to your own types with [`#[dto]`](macro@crate::dto); the scalars and containers below are
/// covered already.
///
/// `Dto` is a *handler-side* contract: it gates what a handler may put on the wire, not what the
/// generated client can decode. A few impls below (`&T`, [`http::StatusCode`]) exist so a handler
/// returning a borrowed or status-only response still compiles — but such a response cannot be
/// decoded into by a typed client, so that one route's generated client method is simply uncallable
/// (its `Decodes` bound is unmet), while the rest of the controller's client works. This is
/// deliberate: it keeps "some APIs return plaintext" from forcing every response through JSON.
pub trait Dto {}

/// The unit type — a no-body request or an empty response.
impl Dto for () {}

macro_rules! dto_scalars {
    ($($ty:ty),* $(,)?) => {
        $(impl Dto for $ty {})*
    };
}

// The scalar types a route path/query segment commonly deserializes into.
dto_scalars!(
    String, char, bool, f32, f64, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize,
);

/// An optional wire value (an absent query parameter, a nullable field).
impl<T: Dto> Dto for Option<T> {}

/// A repeated wire value (a multi-valued query parameter, a JSON array body).
impl<T: Dto> Dto for Vec<T> {}

/// A borrowed response value — some handlers return `&str` (or other `&T`) plaintext. It is allowed
/// as a wire type so the handler compiles; the *typed client* still can't decode a response into a
/// borrow, so that one route's client method is simply uncallable (its `Decodes` bound is unmet),
/// while the rest of the controller's client works. Unconditional, so `&str` (`str: !Dto`) is covered.
impl<T: ?Sized> Dto for &T {}

/// A status-only response (`StatusCode`, no body) — like a borrowed response, it is a legitimate
/// handler return that the typed client can't decode into, so that route's client method is
/// uncallable while the rest of the controller works. Gated on `client` (which pulls in `http`, and
/// is when the wire-type assertion fires at all).
#[cfg(feature = "client")]
impl Dto for http::StatusCode {}

macro_rules! dto_tuples {
    ($($t:ident),+) => {
        // A multi-segment path `Path<(A, B, ..)>` is a `Dto` when each element is — the handler
        // assertion checks the whole tuple, and the client splits it into one typed arg per element.
        impl<$($t: Dto),+> Dto for ($($t,)+) {}
    };
}

dto_tuples!(A);
dto_tuples!(A, B);
dto_tuples!(A, B, C);
dto_tuples!(A, B, C, D);
dto_tuples!(A, B, C, D, E);
dto_tuples!(A, B, C, D, E, F);
