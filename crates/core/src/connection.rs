use std::{
    any::{Any, TypeId},
    collections::HashMap,
    future::Future,
    pin::Pin,
};

use overseer_transport::PeerInfo;

/// Per-connection context accessible to RPC handlers.
///
/// Acts as a typed extension map: users insert their own types on connection
/// establishment (via `ConnectionHandler::on_connect`) and retrieve them
/// inside handlers via `get<T>()`.
pub struct ConnectionInfo {
    peer: PeerInfo,
    extensions: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl ConnectionInfo {
    pub fn new(peer: PeerInfo) -> Self {
        Self {
            peer,
            extensions: HashMap::new(),
        }
    }

    pub fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    /// Inserts a value for type `T`, replacing any previous value of that type.
    pub fn insert<T: Any + Send + Sync + 'static>(&mut self, value: T) {
        self.extensions.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Returns a reference to the value for type `T`, if present.
    pub fn get<T: Any + Send + Sync + 'static>(&self) -> Option<&T> {
        self.extensions.get(&TypeId::of::<T>())?.downcast_ref()
    }

    /// Returns a mutable reference to the value for type `T`, if present.
    pub fn get_mut<T: Any + Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.extensions.get_mut(&TypeId::of::<T>())?.downcast_mut()
    }
}

/// Called once per accepted connection, before the first RPC call is dispatched.
///
/// Implementations populate `ConnectionInfo` with any connection-scoped data
/// (auth context, rate-limit state, per-connection DB handles, etc.).
/// Multiple handlers are run in registration order.
///
/// The manual `Pin<Box<...>>` return is required for object safety so that
/// multiple handlers of different concrete types can be stored together.
pub trait ConnectionHandler: Send + Sync + 'static {
    fn on_connect<'a>(
        &'a self,
        info: &'a mut ConnectionInfo,
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'a>>;
}
