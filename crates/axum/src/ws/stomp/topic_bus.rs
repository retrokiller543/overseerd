//! Shared STOMP topic bus, injectable from any axum handler.
//!
//! The bus owns the broker used by a [`Stomp`](super::Stomp) endpoint. It is a framework-provided
//! singleton, so ordinary HTTP controllers and STOMP message controllers publish through the same
//! broker and reach the same live subscriptions.

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

use super::body::{StompBody, Topic};
use super::broker::Broker;

/// The fan-out concurrency used by the ergonomic [`publish`](StompTopicBus::publish) /
/// [`Publisher::publish`](super::Publisher::publish) — a sensible default for typical subscriber
/// counts. Use `publish_to::<N>` to pick an explicit fan-out.
pub const DEFAULT_PUBLISH_FANOUT: usize = 16;

/// Stable component id for the framework-provided STOMP topic bus.
pub const STOMP_TOPIC_BUS_ID: &str = "overseerd:axum:stomp-topic-bus";

/// Display name for the framework-provided STOMP topic bus.
pub const STOMP_TOPIC_BUS_NAME: &str = "StompTopicBus";

/// A broker-backed publish handle shared by every STOMP endpoint and every axum request.
#[derive(Clone)]
pub struct StompTopicBus {
    broker: Arc<Broker>,
}

impl StompTopicBus {
    /// Creates an empty topic bus.
    pub fn new() -> Self {
        Self {
            broker: Arc::new(Broker::new()),
        }
    }

    /// Returns the shared broker backing this bus.
    pub fn broker(&self) -> &Arc<Broker> {
        &self.broker
    }

    /// Fire-and-forget publish of a typed topic value to every current subscriber: encodes the
    /// payload and fans it out synchronously. Returns only the encode error; the broadcast itself
    /// never blocks or fails (the broker delivers into each subscriber's buffer without awaiting, so
    /// a slow/dead subscriber is the broker's concern, not the publisher's) — hence no `async`.
    ///
    /// Use [`publish`](Self::publish) when you need to *know* the message reached every live
    /// subscriber's buffer (backpressure) rather than being dropped for a full one.
    pub fn emit<T: Topic>(&self, topic: T) -> Result<(), CodecError> {
        let body = topic.encode()?;

        self.publish_raw(&topic.destination(), &body);

        Ok(())
    }

    /// Awaited publish with **backpressure**, at the [default fan-out](DEFAULT_PUBLISH_FANOUT).
    /// Unlike [`emit`](Self::emit), when this resolves the `MESSAGE` is committed to every still-live
    /// subscriber's buffer — a slow consumer makes this wait instead of losing the message. Reach
    /// for [`publish_to`](Self::publish_to) to tune the fan-out concurrency.
    pub async fn publish<T: Topic>(&self, topic: T) -> Result<(), CodecError> {
        self.publish_to::<DEFAULT_PUBLISH_FANOUT, T>(topic).await
    }

    /// Awaited publish with **backpressure**, fanning out to up to `N` subscribers concurrently.
    /// Like [`publish`](Self::publish) but with an explicit fan-out concurrency (`N = 1` is
    /// sequential); pick it for the subscriber count you expect.
    pub async fn publish_to<const N: usize, T: Topic>(&self, topic: T) -> Result<(), CodecError> {
        let body = topic.encode()?;

        self.broker
            .deliver::<N>(&topic.destination(), &body, &[])
            .await;

        Ok(())
    }

    /// Publishes an already-encoded STOMP body to `destination`.
    pub fn publish_raw(&self, destination: &str, body: &StompBody) {
        self.broker.publish(destination, body, &[]);
    }
}

impl Default for StompTopicBus {
    fn default() -> Self {
        Self::new()
    }
}

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
