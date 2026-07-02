//! [`Publisher<T>`]: the typed, injected publish surface.
//!
//! A handler takes `Inject<Publisher<ChatTopics>>` and calls `publish(ChatTopics::Room(msg))`. The
//! destination and body come from the [`Topic`] value itself, so a wrong payload type for a topic
//! is a compile error, not a runtime mismatch. `Publisher<T>` is not a seeded component — it is a
//! [`FromContainer`] that resolves the per-message [`StompSession`] and wraps its broker, so it
//! works for any topic-set `T` without a per-`T` descriptor.

use std::marker::PhantomData;

use overseerd_di::{ComponentConstructionContext, DependencyDescriptor, FromContainer};
use overseerd_transport::CodecError;

use super::body::Topic;
use super::headers::StompSession;

/// Publishes typed [`Topic`] values to the broker. Generic over a topic-set `T` (an enum via
/// `#[topics]`), so `publish` only accepts that set's variants.
pub struct Publisher<T: Topic> {
    session: StompSession,
    _marker: PhantomData<fn(T)>,
}

impl<T: Topic> Publisher<T> {
    /// Fans a topic value out to its subscribers: reads its destination and serializes its payload
    /// (both from the [`Topic`] impl), then publishes through the broker.
    pub async fn publish(&self, topic: T) -> Result<(), CodecError> {
        let body = topic.encode()?;

        self.session.publish(topic.destination(), &body);

        Ok(())
    }
}

impl<T> FromContainer for Publisher<T>
where
    T: Topic + Send + Sync + 'static,
{
    fn dependency() -> DependencyDescriptor {
        // A `Publisher<T>` needs only the per-message session; report that edge so DI validation and
        // ordering see the real dependency (not the phantom `T`).
        <StompSession as FromContainer>::dependency()
    }

    async fn from_container(cx: &ComponentConstructionContext) -> overseerd_di::Result<Self> {
        let session = <StompSession as FromContainer>::from_container(cx).await?;

        Ok(Self {
            session,
            _marker: PhantomData,
        })
    }
}

/// Under `di-check`, a `Publisher<T>` resolves from the framework-seeded session, so the
/// compile-time checker treats it as always provided (for any topic set `T`).
#[cfg(feature = "di-check")]
impl<T> overseerd_di::Provide<Publisher<T>> for overseerd_di::Wiring where T: Topic {}
