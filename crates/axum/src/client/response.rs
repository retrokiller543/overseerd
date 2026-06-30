//! The HTTP response envelope.

use std::ops::Deref;

use http::{HeaderMap, StatusCode};

/// The HTTP response envelope: the decoded body `R` plus the status and headers.
///
/// It [`Deref`]s and [`AsRef`]s into the body, so the body reads through transparently
/// (`response.field`, `&*response`) while [`status`](Self::status) and
/// [`headers`](Self::headers) expose the rest. Bundled clients return this envelope for
/// successful HTTP statuses; non-success statuses are surfaced as
/// [`ClientError::Remote`](overseerd_client::ClientError::Remote) with the raw error body.
pub struct HttpResponse<R> {
    status: StatusCode,
    headers: HeaderMap,
    body: R,
}

impl<R> HttpResponse<R> {
    /// Wraps a decoded body with its status and headers.
    pub fn new(status: StatusCode, headers: HeaderMap, body: R) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    /// The HTTP status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// A reference to the decoded body.
    pub fn body(&self) -> &R {
        &self.body
    }

    /// Consumes the envelope, returning the decoded body.
    pub fn into_body(self) -> R {
        self.body
    }
}

impl<R> Deref for HttpResponse<R> {
    type Target = R;

    fn deref(&self) -> &R {
        &self.body
    }
}

impl<R> AsRef<R> for HttpResponse<R> {
    fn as_ref(&self) -> &R {
        &self.body
    }
}
