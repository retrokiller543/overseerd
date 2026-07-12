//! STOMP CONNECT authentication and the principal it establishes for message handlers.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use overseerd_di::Injectable;

use super::StompHeaders;

/// The parsed authentication-relevant contents of a STOMP `CONNECT` frame.
///
/// The custom headers are preserved in wire order. `Debug` deliberately redacts the passcode.
#[derive(Clone)]
pub struct StompConnect {
    host: String,
    login: Option<String>,
    passcode: Option<String>,
    headers: StompHeaders,
}

impl StompConnect {
    pub(crate) fn new(
        host: String,
        login: Option<String>,
        passcode: Option<String>,
        headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            host,
            login,
            passcode,
            headers: StompHeaders::new(headers),
        }
    }

    /// The virtual host requested by the client.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The standard STOMP `login` credential, if supplied.
    pub fn login(&self) -> Option<&str> {
        self.login.as_deref()
    }

    /// The standard STOMP `passcode` credential, if supplied.
    pub fn passcode(&self) -> Option<&str> {
        self.passcode.as_deref()
    }

    /// Custom CONNECT headers, for token or application-specific authentication schemes.
    pub fn headers(&self) -> &StompHeaders {
        &self.headers
    }
}

impl fmt::Debug for StompConnect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self.headers.iter().map(|(name, _)| name).collect();

        f.debug_struct("StompConnect")
            .field("host", &self.host)
            .field("login", &self.login)
            .field("passcode", &self.passcode.as_ref().map(|_| "[REDACTED]"))
            .field("header_names", &header_names)
            .finish()
    }
}

/// The authenticated identity attached to every message on a STOMP connection.
///
/// An endpoint without an authenticator receives [`anonymous`](Self::anonymous) principals. An
/// authenticator returns [`authenticated`](Self::authenticated) with an application-defined
/// subject and may attach string attributes for authorization decisions in message handlers.
#[derive(Clone, Debug, Default)]
pub struct StompPrincipal {
    subject: Option<Arc<str>>,
    attributes: Arc<HashMap<String, String>>,
}

impl StompPrincipal {
    /// An unauthenticated principal used when an endpoint has no authenticator.
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// An authenticated principal identified by `subject`.
    pub fn authenticated(subject: impl Into<Arc<str>>) -> Self {
        Self {
            subject: Some(subject.into()),
            attributes: Arc::default(),
        }
    }

    /// Adds or replaces one authorization attribute.
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.attributes).insert(name.into(), value.into());

        self
    }

    /// Whether an authenticator established this identity.
    pub fn is_authenticated(&self) -> bool {
        self.subject.is_some()
    }

    /// The authenticated subject, or `None` for an anonymous connection.
    pub fn subject(&self) -> Option<&str> {
        self.subject.as_deref()
    }

    /// One application-defined authorization attribute.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }

    /// Every application-defined authorization attribute.
    pub fn attributes(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attributes
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

impl Injectable for StompPrincipal {
    type Target = StompPrincipal;
    type Stored = Self;

    fn into_stored(self) -> Self {
        self
    }

    fn from_stored(stored: &Self) -> Self {
        stored.clone()
    }
}

/// A rejected STOMP connection.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct StompAuthenticationError {
    message: String,
}

impl StompAuthenticationError {
    /// Builds a rejection with a client-visible error message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// The boxed authentication future returned by [`StompAuthenticator`].
pub type StompAuthFuture = Pin<
    Box<dyn Future<Output = Result<StompPrincipal, StompAuthenticationError>> + Send + 'static>,
>;

/// Authenticates one parsed STOMP `CONNECT` frame before the broker sends `CONNECTED`.
///
/// Async closures implement this trait directly, so most applications configure authentication
/// without a named type:
///
/// ```ignore
/// StompConfig::default().with_authenticator(|connect: StompConnect| async move {
///     validate(connect.login(), connect.passcode()).await?;
///     Ok(StompPrincipal::authenticated(connect.login().unwrap()))
/// })
/// ```
pub trait StompAuthenticator: Send + Sync + 'static {
    /// Resolves to a principal on success or rejects the connection before `CONNECTED`.
    fn authenticate(&self, connect: StompConnect) -> StompAuthFuture;
}

impl<F, Fut> StompAuthenticator for F
where
    F: Fn(StompConnect) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<StompPrincipal, StompAuthenticationError>> + Send + 'static,
{
    fn authenticate(&self, connect: StompConnect) -> StompAuthFuture {
        Box::pin(self(connect))
    }
}

#[cfg(feature = "di-check")]
impl overseerd_di::Provide<StompPrincipal> for overseerd_di::Wiring {}

#[cfg(test)]
mod tests;
