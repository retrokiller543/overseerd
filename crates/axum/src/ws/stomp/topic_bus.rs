//! Shared topic bus, injectable from any axum handler.
//!
//! [`TopicBus<P>`] owns the [`SubscriptionRegistry`] used by a pub/sub endpoint, generic over the
//! protocol `P`. It is a framework-provided singleton, so ordinary HTTP controllers and message
//! controllers publish through the same registry and reach the same live subscriptions. STOMP's
//! instantiation is [`StompTopicBus`] (`TopicBus<Stomp>`), registered as a DI singleton; a new
//! protocol registers its own `TopicBus<ThatProtocol>` descriptor the same way.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use overseerd_core::{DependencyDescriptor, Singleton, TypeDescriptor};
use overseerd_di::{
    BoxedComponent, Component, ComponentConstructionContext, ComponentDescriptor,
    ComponentFactoryDescriptor, Injectable,
};
use overseerd_hooks::no_hooks;
use overseerd_transport::CodecError;

use crate::stomp::{Stomp, Topic};
use crate::ws::PubSubProtocol;

use super::broker::{Broker, SubscriptionRegistry};

/// The fan-out concurrency used by the ergonomic [`publish`](TopicBus::publish) /
/// [`Publisher::publish`](super::Publisher::publish) — a sensible default for typical subscriber
/// counts. Use `publish_to::<N>` to pick an explicit fan-out.
pub const DEFAULT_PUBLISH_FANOUT: usize = 16;

/// Stable component id for the framework-provided STOMP topic bus.
pub const STOMP_TOPIC_BUS_ID: &str = "overseerd:axum:stomp-topic-bus";

/// Display name for the framework-provided STOMP topic bus.
pub const STOMP_TOPIC_BUS_NAME: &str = "StompTopicBus";

/// A registry-backed publish handle shared by every endpoint and every axum request for protocol
/// `P`. Cheap to clone (an `Arc` onto the shared registry).
pub struct TopicBus<P: PubSubProtocol> {
    registry: Arc<SubscriptionRegistry<P::OutFrame>>,
}

impl<P: PubSubProtocol> Clone for TopicBus<P> {
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
        }
    }
}

impl<P: PubSubProtocol> TopicBus<P> {
    /// Creates an empty topic bus.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(SubscriptionRegistry::new()),
        }
    }

    /// The shared subscription registry backing this bus.
    pub fn registry(&self) -> &Arc<SubscriptionRegistry<P::OutFrame>> {
        &self.registry
    }

    /// Fire-and-forget publish of a typed topic value to every current subscriber: encodes the
    /// payload and fans it out synchronously. Returns only the encode error; the broadcast itself
    /// never blocks or fails (the registry delivers into each subscriber's buffer without awaiting,
    /// so a slow/dead subscriber is the registry's concern, not the publisher's) — hence no `async`.
    ///
    /// Use [`publish`](Self::publish) when you need to *know* the message reached every live
    /// subscriber's buffer (backpressure) rather than being dropped for a full one.
    pub fn emit<T>(&self, topic: T) -> Result<(), CodecError>
    where
        T: Topic<Protocol = P>,
    {
        let body = topic.encode()?;
        let destination = topic.destination();

        self.publish_raw(destination.as_ref(), &body);

        Ok(())
    }

    /// Awaited publish with **backpressure**, at the [default fan-out](DEFAULT_PUBLISH_FANOUT).
    /// Unlike [`emit`](Self::emit), when this resolves the message is committed to every still-live
    /// subscriber's buffer — a slow consumer makes this wait instead of losing the message. Reach
    /// for [`publish_to`](Self::publish_to) to tune the fan-out concurrency.
    pub async fn publish<T>(&self, topic: T) -> Result<(), CodecError>
    where
        T: Topic<Protocol = P>,
    {
        self.publish_to::<DEFAULT_PUBLISH_FANOUT, T>(topic).await
    }

    /// Awaited publish with **backpressure**, fanning out to up to `N` subscribers concurrently.
    /// Like [`publish`](Self::publish) but with an explicit fan-out concurrency (`N = 1` is
    /// sequential); pick it for the subscriber count you expect.
    pub async fn publish_to<const N: usize, T>(&self, topic: T) -> Result<(), CodecError>
    where
        T: Topic<Protocol = P>,
    {
        let body = topic.encode()?;
        let destination = topic.destination();

        self.registry
            .deliver_frames::<N>(destination.as_ref(), |sub_id, message_id| {
                P::frame_message(message_id, destination.as_ref(), sub_id, &body, &[])
            })
            .await;

        Ok(())
    }

    /// Publishes an already-encoded body to `destination`.
    pub fn publish_raw(&self, destination: &str, body: &P::Body) {
        self.registry
            .publish_frames(destination, |sub_id, message_id| {
                P::frame_message(message_id, destination, sub_id, body, &[])
            });
    }
}

impl<P: PubSubProtocol> Default for TopicBus<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl TopicBus<Stomp> {
    /// The shared broker backing this STOMP bus (its subscription registry). Kept for the STOMP
    /// serve loop, which registers connections and publishes directly through it.
    pub fn broker(&self) -> &Arc<Broker> {
        &self.registry
    }
}

/// STOMP's [`TopicBus`]: the framework-provided singleton for REST-triggered and STOMP-triggered
/// topic publishes. A new protocol aliases and registers its own `TopicBus<ThatProtocol>`.
pub type StompTopicBus = TopicBus<Stomp>;

impl Component for StompTopicBus {
    const ID: &'static str = STOMP_TOPIC_BUS_ID;
    const NAME: &'static str = STOMP_TOPIC_BUS_NAME;
    type Handle = StompTopicBus;

    fn into_handle(self) -> Self::Handle {
        self
    }
}

impl Injectable for StompTopicBus {
    type Target = StompTopicBus;
    type Stored = Self;

    fn into_stored(self) -> Self {
        self
    }

    fn from_stored(stored: &Self) -> Self {
        stored.clone()
    }
}

fn topic_bus_dependencies() -> Vec<DependencyDescriptor> {
    Vec::new()
}

fn construct_topic_bus<'a>(
    _: &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseerd_di::Result<BoxedComponent>> + Send + 'a>> {
    Box::pin(async {
        Ok(BoxedComponent {
            ty: TypeDescriptor::of::<StompTopicBus>(STOMP_TOPIC_BUS_NAME),
            value: Box::new(Injectable::into_stored(StompTopicBus::new().into_handle())),
        })
    })
}

static STOMP_TOPIC_BUS_FACTORIES: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: construct_topic_bus,
    dependencies: topic_bus_dependencies,
    default: true,
}];

fn topic_bus_factories() -> &'static [ComponentFactoryDescriptor] {
    &STOMP_TOPIC_BUS_FACTORIES
}

/// Framework-provided singleton for REST-triggered and STOMP-triggered topic publishes.
pub(crate) static STOMP_TOPIC_BUS_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor {
    id: STOMP_TOPIC_BUS_ID,
    name: STOMP_TOPIC_BUS_NAME,
    ty: TypeDescriptor::of::<StompTopicBus>(STOMP_TOPIC_BUS_NAME),
    scope: &Singleton,
    factories: topic_bus_factories,
    hooks: no_hooks,
};

#[cfg(feature = "di-check")]
impl overseerd_di::Provide<StompTopicBus> for overseerd_di::Wiring {}
