//! The generated HTTP client's runtime: the request body family, the response envelope, and
//! pluggable transport backends.
//!
//! A generated `{Controller}Client<C>` is transport-generic: `C` is any backend implementing
//! the [`Unary`](overseerd_client::Unary) capability with `Request<B> = http::Request<B>` and
//! `Response<R> = HttpResponse<R>` (plus the [`Encodes`](overseerd_transport::Encodes) /
//! [`Decodes`](overseerd_transport::Decodes) codec). Both bundled backends — [`reqwest`] and
//! `hyper` — qualify, so the same client runs over either; pick one with the matching feature.

mod body;
mod response;

#[cfg(feature = "hyper")]
mod hyper_backend;
#[cfg(feature = "reqwest")]
mod reqwest_backend;

pub use body::{HttpBody, OctetStream};
pub use response::HttpResponse;

#[cfg(feature = "hyper")]
pub use hyper_backend::HyperClient;
#[cfg(feature = "reqwest")]
pub use reqwest_backend::ReqwestClient;
