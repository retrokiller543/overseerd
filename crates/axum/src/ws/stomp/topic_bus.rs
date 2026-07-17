//! STOMP's DI-registered topic bus.
//!
//! [`StompTopicBus`] is `TopicBus<Stomp>` — the protocol-generic [`TopicBus`](crate::ws::pubsub::TopicBus)
//! (in [`crate::ws::pubsub`]) instantiated for STOMP and registered as a framework DI singleton, so
//! ordinary HTTP controllers and STOMP message controllers publish through the same registry and
//! reach the same live subscriptions. A new protocol registers its own `TopicBus<ThatProtocol>`
//! descriptor the same way.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use overseerd_core::{Singleton, TypeDescriptor};
use overseerd_di::{
    BoxedComponent, Component, ComponentConstructionContext, ComponentDescriptor,
    ComponentFactoryDescriptor, DependencyDescriptor, Injectable,
};
use overseerd_hooks::no_hooks;

use crate::stomp::Stomp;
use crate::ws::pubsub::TopicBus;

use super::broker::Broker;

/// Stable component id for the framework-provided STOMP topic bus.
pub const STOMP_TOPIC_BUS_ID: &str = "overseerd:axum:stomp-topic-bus";

/// Display name for the framework-provided STOMP topic bus.
pub const STOMP_TOPIC_BUS_NAME: &str = "StompTopicBus";

impl TopicBus<Stomp> {
    /// The shared broker backing this STOMP bus (its subscription registry). Kept for the STOMP
    /// serve loop, which registers connections and publishes directly through it.
    pub fn broker(&self) -> &Arc<Broker> {
        self.registry()
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
